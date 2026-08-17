#!/usr/bin/env bash
# Fetch pre-built documentation screenshots from the deployed website instead of
# regenerating them from source (todoer #1389, #1446). Push/merge website builds
# use this so the docs aren't recaptured on every push — only the nightly (or an
# explicit workflow_dispatch) re-captures them.
#
# The nightly publishes `manifest.txt` (one relative path per line, relative to
# img/screenshots/) alongside the PNGs. This script reads that manifest and
# downloads each listed asset from the live site. Anything that fails to
# download is skipped with a warning and never fails the build. The nightly
# also publishes `screenshots-commit.txt`, which the Website plan job reads to
# decide whether the repo changed since the last nightly.
#
# Dotfile names (`.manifest`, `.screenshots-commit`) are accepted as a fallback
# when reading an old deploy, but they are not written: Docusaurus does not
# copy dotfiles out of `static/`, so those markers never reached the live site
# and every push used to fall back to a full screenshot rebuild (#1446).
#
# Usage:
#   scripts/fetch-doc-screenshots.sh                              # defaults below
#   PAGES_BASE="https://iffy.github.io/BearCAD/" scripts/fetch-doc-screenshots.sh
#
# Env:
#   BEARCAD_SCREENSHOT_OUT  destination PNG dir (default: docs-site/static/img/screenshots)
#   PAGES_BASE              deployed site base URL with trailing slash (required)
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT_DIR="${BEARCAD_SCREENSHOT_OUT:-docs-site/static/img/screenshots}"
PAGES_BASE="${PAGES_BASE:-}"

mkdir -p "$OUT_DIR"

if [[ -z "$PAGES_BASE" ]]; then
  echo "error: PAGES_BASE (deployed site base URL) is required" >&2
  exit 2
fi
PAGES_BASE="${PAGES_BASE%/}/"

is_meta() {
  case "$1" in
    manifest.txt|.manifest|screenshots-commit.txt|.screenshots-commit) return 0 ;;
    *) return 1 ;;
  esac
}

write_local_manifest() {
  find "$OUT_DIR" -type f 2>/dev/null \
    | sed "s|^$OUT_DIR/||" \
    | while IFS= read -r rel; do
        is_meta "$rel" && continue
        printf '%s\n' "$rel"
      done \
    | LC_ALL=C sort \
    > "$OUT_DIR/manifest.txt"
  rm -f "$OUT_DIR/.manifest"
}

# Prefer the names Docusaurus actually deploys; keep the old dotfiles as fallback.
fetch_text() {
  local url="$1"
  local body
  body="$(curl -fsSL --max-time 30 "$url" 2>/dev/null)" || return 1
  # A Docusaurus SPA 404 is HTML; treat it as missing.
  if [[ "$body" == *"<html"* || "$body" == *"<!DOCTYPE"* ]]; then
    return 1
  fi
  printf '%s' "$body"
}

MANIFEST_URL="${PAGES_BASE}img/screenshots/manifest.txt"
OLD_MANIFEST_URL="${PAGES_BASE}img/screenshots/.manifest"
manifest="$(fetch_text "$MANIFEST_URL" || fetch_text "$OLD_MANIFEST_URL" || true)"

# Keep the nightly commit marker across push deploys so later nightlies can skip.
marker="$(
  fetch_text "${PAGES_BASE}img/screenshots/screenshots-commit.txt" \
    || fetch_text "${PAGES_BASE}img/screenshots/.screenshots-commit" \
    || true
)"
if [[ -n "$marker" ]]; then
  printf '%s\n' "$(printf '%s' "$marker" | tr -d '[:space:]')" > "$OUT_DIR/screenshots-commit.txt"
fi
rm -f "$OUT_DIR/.screenshots-commit"

if [[ -z "$manifest" ]]; then
  echo "warning: could not fetch screenshot manifest from $MANIFEST_URL;" >&2
  echo "         guessing names from screenshot scripts and docs references." >&2
  manifest="$(
    {
      shopt -s nullglob
      for script in docs-site/screenshots/*.lua; do
        printf '%s.png\n' "$(basename "$script" .lua)"
      done
      grep -RhoaE '/img/screenshots/[A-Za-z0-9._-]+' docs-site 2>/dev/null \
        | sed 's|^/img/screenshots/||' || true
    } | LC_ALL=C sort -u
  )"
  if [[ -z "$manifest" ]]; then
    echo "warning: no names to fetch; keeping whatever is already in $OUT_DIR." >&2
    write_local_manifest
    if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
      echo "skipped=0"  >> "$GITHUB_OUTPUT"
      echo "downloaded=0"  >> "$GITHUB_OUTPUT"
      echo "manifest_total=0" >> "$GITHUB_OUTPUT"
    fi
    exit 0
  fi
fi

manifest_total="$(printf '%s\n' "$manifest" | grep -c . || true)"

downloaded=0
skipped=0
while IFS= read -r rel; do
  [[ -z "$rel" ]] && continue
  # Defence-in-depth: strip any leading slash or redundant prefix.
  rel="${rel#/}"
  rel="${rel#img/screenshots/}"
  [[ -z "$rel" ]] && continue
  is_meta "$rel" && continue
  dest="$OUT_DIR/$rel"
  mkdir -p "$(dirname "$dest")"
  if curl -fsSL --max-time 60 "${PAGES_BASE}img/screenshots/${rel}" -o "$dest" 2>/dev/null; then
    downloaded=$((downloaded + 1))
  else
    echo "warning: could not fetch $rel from site; omitting (missing screenshot)." >&2
    skipped=$((skipped + 1))
    rm -f "$dest"
  fi
done <<< "$manifest"

# Regenerate the manifest from what we actually have so the (re)deployed site
# still advertises the same file list for the next push to fetch from (#1389).
write_local_manifest

echo "Fetched $downloaded screenshot asset(s) from $PAGES_BASE; $skipped missing/omitted (not a failure)."

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "skipped=$skipped"       >> "$GITHUB_OUTPUT"
  echo "downloaded=$downloaded" >> "$GITHUB_OUTPUT"
  echo "manifest_total=$manifest_total" >> "$GITHUB_OUTPUT"
fi
