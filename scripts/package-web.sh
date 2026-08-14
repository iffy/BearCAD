#!/usr/bin/env bash
# Package the built web app (web/dist/) into bearcad-web.zip for release attachment.
# Usage: scripts/package-web.sh [outfile]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${1:-web/bearcad-web.zip}"
if [[ -z "$(ls -A web/dist 2>/dev/null)" ]]; then
  echo "error: web/dist is empty; run scripts/build-web.sh first" >&2
  exit 1
fi

rm -f "$OUT"
(cd web/dist && zip -rq "../$(basename "$OUT")" .)
echo "Created $OUT:"
ls -la "$OUT"
