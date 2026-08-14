#!/usr/bin/env bash
# Fetch pre-built documentation screenshots from the deployed website instead of
# regenerating them from source (todoer #1389). Push/merge website builds use this
# so the docs aren't rebuilt on every push — only the nightly job re-captures them.
#
# The nightly build publishes a `.manifest` (one whitespace-free relative path per
# line, relative to img/screenshots/) alongside the PNGs. This script reads that
# manifest and downloads each listed asset from the live site. Anything that fails
# to download — a still-missing screenshot, an asset removed from the site, or an
# empty first deploy with no manifest yet — is skipped with a warning and never
# fails the build. The nightly also publishes `.screenshots-commit`, which the CI
# plan job reads to decide whether the repo changed since the last nightly.
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

MANIFEST_URL="${PAGES_BASE}img/screenshots/.manifest"
manifest="$(curl -fsSL --max-time 30 "$MANIFEST_URL" 2>/dev/null)" || {
  echo "warning: could not fetch screenshot manifest from $MANIFEST_URL;" >&2
  echo "         deploying with only whatever is already in $OUT_DIR (missing is OK)." >&2
  exit 0
}

downloaded=0
skipped=0
while IFS= read -r rel; do
  [[ -z "$rel" ]] && continue
  # Defence-in-depth: strip any leading slash or redundant prefix.
  rel="${rel#/}"
  rel="${rel#img/screenshots/}"
  [[ -z "$rel" ]] && continue
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

# Regenerate the manifest from what we actually fetched so the (re)deployed site
# still advertises the same file list for the next push to fetch from (#1389).
find "$OUT_DIR" -type f 2>/dev/null \
  | sed "s|^$OUT_DIR/||" \
  | LC_ALL=C sort \
  > "$OUT_DIR/.manifest"

echo "Fetched $downloaded screenshot asset(s) from $PAGES_BASE; $skipped missing/omitted (not a failure)."
