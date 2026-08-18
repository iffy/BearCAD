#!/usr/bin/env bash
# Package release artifacts for CI and local builds.
# Usage: scripts/package-release.sh <linux|macos|windows>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

version() {
  grep '^version' Cargo.toml | head -1 | cut -d'"' -f2
}

package_linux() {
  local out="bearcad-linux-x86_64.tar.gz"
  rm -rf dist
  mkdir -p dist
  cp target/release/bearcad dist/
  # FreeDesktop association files (#1285). `bearcad install-cli` (or first GUI launch)
  # installs copies under ~/.local/share with the real Exec= path; these are the templates
  # shipped in the tarball so packagers / manual installs have something to place.
  cat >dist/com.bearcad.app.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=BearCAD
Comment=Parametric CAD
Exec=bearcad %F
Icon=bearcad
Terminal=false
Categories=Graphics;Engineering;Science;
MimeType=application/x-bearcad;
StartupWMClass=bearcad
EOF
  cat >dist/com.bearcad.app.xml <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-bearcad">
    <comment>BearCAD document</comment>
    <glob pattern="*.bearcad"/>
  </mime-type>
</mime-info>
EOF
  tar czvf "$out" -C dist bearcad com.bearcad.app.desktop com.bearcad.app.xml
  echo "Created $out"
}

package_macos() {
  local version app_name app_dir dmg
  version="$(version)"
  app_name="BearCAD"
  app_dir="dist/${app_name}.app"
  dmg="bearcad.dmg"

  rm -rf dist
  mkdir -p "${app_dir}/Contents/MacOS" "${app_dir}/Contents/Resources"
  cp target/release/bearcad "${app_dir}/Contents/MacOS/bearcad"
  chmod +x "${app_dir}/Contents/MacOS/bearcad"
  bash scripts/generate-macos-icns.sh dist/AppIcon.icns
  cp dist/AppIcon.icns "${app_dir}/Contents/Resources/AppIcon.icns"

  cat >"${app_dir}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>bearcad</string>
  <key>CFBundleIdentifier</key>
  <string>com.bearcad.app</string>
  <key>CFBundleName</key>
  <string>${app_name}</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key>
      <string>BearCAD Document</string>
      <key>CFBundleTypeRole</key>
      <string>Editor</string>
      <key>LSHandlerRank</key>
      <string>Owner</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>com.bearcad.document</string>
      </array>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>bearcad</string>
      </array>
    </dict>
  </array>
  <key>UTExportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key>
      <string>com.bearcad.document</string>
      <key>UTTypeDescription</key>
      <string>BearCAD Document</string>
      <key>UTTypeConformsTo</key>
      <array>
        <string>public.data</string>
      </array>
      <key>UTTypeTagSpecification</key>
      <dict>
        <key>public.filename-extension</key>
        <array>
          <string>bearcad</string>
        </array>
        <key>public.mime-type</key>
        <string>application/x-bearcad</string>
      </dict>
    </dict>
  </array>
</dict>
</plist>
EOF

  # QuickLook Preview Extension (#1290): Space-bar preview of .bearcad with SceneKit
  # rotate/pan/zoom, same gesture as system STL. Reads the embedded preview_stl meta.
  mkdir -p "${app_dir}/Contents/PlugIns"
  bash scripts/build-macos-quicklook.sh "${app_dir}/Contents/PlugIns/BearCADQuickLook.appex"

  # Developer ID + notarize + staple when a signing identity is available
  # (CI); otherwise ad-hoc sign so Apple Silicon will still launch the bundle.
  # See scripts/sign-macos.sh.
  bash scripts/sign-macos.sh sign-app "$app_dir"

  # Stage the .dmg contents: the app plus an /Applications symlink so the user can
  # drag BearCAD.app straight into Applications from the mounted volume.
  local stage="dist/dmg"
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -R "$app_dir" "$stage/"
  ln -s /Applications "$stage/Applications"

  rm -f "$dmg"
  make_styled_macos_dmg "$stage" "$app_name" "$dmg"
  bash scripts/sign-macos.sh sign-dmg "$dmg"
  echo "Created $dmg"
}

# Build a compressed .dmg whose Finder window is the classic drag-to-Applications
# layout (#1451): honey background, BearCAD.app on the left, Applications on the right.
# Uses hdiutil + Finder AppleScript (no extra tools). Window is 660×400; the
# committed PNG is 1320×800 and becomes a HiDPI TIFF via tiffutil.
make_styled_macos_dmg() {
  local stage="$1"
  local volname="$2"
  local dmg="$3"
  local bg_src="macos/dmg-background.png"
  local rw_dmg="dist/bearcad-rw.dmg"
  local volume="/Volumes/${volname}"

  if [[ ! -f "$bg_src" ]]; then
    echo "missing $bg_src" >&2
    exit 1
  fi

  # Read-write HFS+ image so Finder can write .DS_Store / background after mount.
  rm -f "$rw_dmg"
  hdiutil create -volname "$volname" -srcfolder "$stage" -ov -format UDRW -fs HFS+ "$rw_dmg"

  # Slack for .DS_Store + the background TIFF. macOS 15+ prints one
  # min/cur/max line (older releases print a header first); take the first
  # numeric current-size column so we don't resize below the image.
  local sectors
  sectors="$(hdiutil resize -limits "$rw_dmg" | awk '$2 ~ /^[0-9]+$/ { print $2; exit }')"
  if [[ -z "$sectors" ]]; then
    echo "failed to parse current sector count from hdiutil resize -limits:" >&2
    hdiutil resize -limits "$rw_dmg" >&2
    exit 1
  fi
  hdiutil resize -sectors "$((sectors + 20000))" "$rw_dmg"

  # Detach any leftover mount from a previous failed run.
  if [[ -d "$volume" ]]; then
    hdiutil detach "$volume" -quiet || true
    sleep 1
  fi

  local attach_out device
  attach_out="$(hdiutil attach -readwrite -noverify -noautoopen "$rw_dmg")"
  device="$(awk '/Apple_HFS|Apple_HFSX|Macintosh HD/ {print $1; exit} /\/Volumes\// {print $1; exit}' <<<"$attach_out")"
  if [[ -z "$device" ]]; then
    echo "failed to attach $rw_dmg" >&2
    echo "$attach_out" >&2
    exit 1
  fi

  local i
  for i in $(seq 1 30); do
    [[ -d "$volume" ]] && break
    sleep 0.2
  done
  if [[ ! -d "$volume" ]]; then
    echo "volume $volume did not appear" >&2
    hdiutil detach "$device" -quiet || true
    exit 1
  fi

  mkdir -p "$volume/.background"
  # 1× + 2× PNG → multi-resolution TIFF so the backdrop is sharp on Retina.
  sips -z 400 660 "$bg_src" --out "$volume/.background/background.png" >/dev/null
  sips -z 800 1320 "$bg_src" --out "$volume/.background/background@2x.png" >/dev/null
  tiffutil -cathidpicheck \
    "$volume/.background/background.png" \
    "$volume/.background/background@2x.png" \
    -out "$volume/.background/background.tiff"
  rm -f "$volume/.background/background.png" "$volume/.background/background@2x.png"

  # Finder writes the window bounds, icon-view options, and background picture
  # into .DS_Store. Icon positions: app left (160, 185), Applications right (500, 185).
  osascript <<EOF
tell application "Finder"
  tell disk "$volname"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {200, 120, 860, 520}
    set theViewOptions to the icon view options of container window
    set arrangement of theViewOptions to not arranged
    set icon size of theViewOptions to 128
    set background picture of theViewOptions to file ".background:background.tiff"
    set position of item "BearCAD.app" of container window to {160, 185}
    set position of item "Applications" of container window to {500, 185}
    close
    open
    update without registering applications
    delay 2
    close
  end tell
end tell
EOF

  # Hide the backdrop folder after Finder has a handle on the TIFF.
  chflags hidden "$volume/.background" || true
  # --openfolder is Intel-only; Apple Silicon prints an error and the volume
  # still auto-opens from the .DS_Store window state.
  bless --folder "$volume" --openfolder "$volume" >/dev/null 2>&1 || true

  sync
  sleep 1
  for i in $(seq 1 10); do
    if hdiutil detach "$device"; then
      break
    fi
    sleep 2
    if [[ "$i" -eq 10 ]]; then
      echo "failed to detach $device" >&2
      exit 1
    fi
  done

  rm -f "$dmg"
  hdiutil convert "$rw_dmg" -format UDZO -imagekey zlib-level=9 -o "$dmg"
  rm -f "$rw_dmg"
}

package_windows() {
  pwsh -NoProfile -File scripts/package-windows.ps1
}

target="${1:-}"
case "$target" in
  linux) package_linux ;;
  macos) package_macos ;;
  windows) package_windows ;;
  # Style an already-staged folder (BearCAD.app + Applications) into a .dmg.
  # Used to test the drag-to-Applications backdrop without a full release build.
  macos-dmg)
    mkdir -p dist
    make_styled_macos_dmg "${2:?stage dir}" "BearCAD" "${3:-bearcad.dmg}"
    ;;
  *)
    echo "usage: $0 <linux|macos|windows>" >&2
    exit 1
    ;;
esac