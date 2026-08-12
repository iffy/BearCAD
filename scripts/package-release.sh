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

  # Ad-hoc code-sign the bundle. Apple Silicon requires every executable to carry
  # at least an ad-hoc signature; an unsigned (or signature-invalidated) bundle that
  # has been quarantined after download is reported by Gatekeeper as "damaged and
  # can't be opened". Signing the assembled bundle deeply produces a valid signature
  # so the app launches (after the user clears quarantine / right-click → Open).
  codesign --force --deep --sign - --timestamp=none "$app_dir"
  codesign --verify --deep --strict "$app_dir"

  # Stage the .dmg contents: the app plus an /Applications symlink so the user can
  # drag BearCAD.app straight into Applications from the mounted volume.
  local stage="dist/dmg"
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -R "$app_dir" "$stage/"
  ln -s /Applications "$stage/Applications"

  rm -f "$dmg"
  hdiutil create -volname "$app_name" -srcfolder "$stage" -ov -format UDZO "$dmg"
  echo "Created $dmg"
}

package_windows() {
  pwsh -NoProfile -File scripts/package-windows.ps1
}

target="${1:-}"
case "$target" in
  linux) package_linux ;;
  macos) package_macos ;;
  windows) package_windows ;;
  *)
    echo "usage: $0 <linux|macos|windows>" >&2
    exit 1
    ;;
esac