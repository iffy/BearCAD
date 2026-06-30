#!/usr/bin/env bash
# Build AppIcon.icns from src/assets/appicon.png (macOS only).
# Usage: scripts/generate-macos-icns.sh [output.icns]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "generate-macos-icns.sh requires macOS (sips + iconutil)" >&2
  exit 1
fi

PNG="src/assets/appicon.png"
OUT="${1:-dist/AppIcon.icns}"
ICONSET="${OUT%.icns}.iconset"

mkdir -p "$(dirname "$OUT")"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"

for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$PNG" --out "${ICONSET}/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$PNG" --out "${ICONSET}/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$ICONSET" -o "$OUT"
rm -rf "$ICONSET"
echo "Created $OUT"