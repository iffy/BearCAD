#!/usr/bin/env bash
# Build the BearCAD QuickLook Preview Extension (.appex) for packaging into BearCAD.app.
# Usage: scripts/build-macos-quicklook.sh [output.appex]
# Requires: macOS, Xcode CLT (swiftc, codesign). No network, no installs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/macos/quicklook"
OUT="${1:-$ROOT/dist/BearCADQuickLook.appex}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-macos-quicklook.sh requires macOS" >&2
  exit 1
fi

SDK="$(xcrun --sdk macosx --show-sdk-path)"
# Match the host arch so the extension loads on this machine (CI is aarch64).
ARCH="$(uname -m)"
# SceneKit/ModelIO/QL need 12+; keep in sync with Info.plist LSMinimumSystemVersion.
MIN_OS="12.0"
TARGET="${ARCH}-apple-macosx${MIN_OS}"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/bearcad-ql.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# Compile the extension binary. App extensions enter via _NSExtensionMain.
# -parse-as-library so we don't need a free-standing main; the principal class is loaded by name.
swiftc \
  -target "$TARGET" \
  -sdk "$SDK" \
  -O \
  -module-name BearCADQuickLook \
  -parse-as-library \
  -emit-executable \
  -o "$WORK/BearCADQuickLook" \
  -framework AppKit \
  -framework QuickLookUI \
  -framework SceneKit \
  -framework ModelIO \
  -framework Quartz \
  -framework UniformTypeIdentifiers \
  -lsqlite3 \
  -Xlinker -e \
  -Xlinker _NSExtensionMain \
  "$SRC/PreviewViewController.swift"

# Assemble the .appex bundle.
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"
cp "$WORK/BearCADQuickLook" "$OUT/Contents/MacOS/BearCADQuickLook"
chmod +x "$OUT/Contents/MacOS/BearCADQuickLook"
cp "$SRC/Info.plist" "$OUT/Contents/Info.plist"

# Ad-hoc sign with sandbox entitlement (required for app extensions).
codesign --force --sign - \
  --entitlements "$SRC/BearCADQuickLook.entitlements" \
  --timestamp=none \
  "$OUT"

codesign --verify --strict "$OUT"
echo "Created $OUT"
