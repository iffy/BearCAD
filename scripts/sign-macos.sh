#!/usr/bin/env bash
# Sign, notarize, and staple BearCAD macOS release artifacts.
#
# Usage:
#   scripts/sign-macos.sh import-keychain
#   scripts/sign-macos.sh delete-keychain
#   scripts/sign-macos.sh sign-app <BearCAD.app>
#   scripts/sign-macos.sh sign-dmg <bearcad.dmg>
#
# Environment:
#   APPLE_CODESIGN_IDENTITY
#       Optional. Exact codesign identity, e.g.
#       "Developer ID Application: Example, Inc. (ABCDE12345)".
#       When unset, the first Developer ID Application identity in the
#       keychain is used. With no such identity, the app is ad-hoc signed
#       (unless BEARCAD_REQUIRE_SIGN=1).
#   APPLE_DEVELOPER_ID_APPLICATION_P12
#       Base64-encoded Developer ID Application .p12 (import-keychain).
#   APPLE_DEVELOPER_ID_APPLICATION_PASSWORD
#       Password for that .p12.
#   APPLE_DEVELOPER_ID_INSTALLER_P12
#       Optional. Base64-encoded Developer ID Installer .p12. Imported if
#       set; the shipped .dmg is signed with the Application identity.
#   APPLE_DEVELOPER_ID_INSTALLER_PASSWORD
#       Password for the installer .p12.
#   APPLE_API_KEY_ID
#       App Store Connect API key id (AuthKey_<id>.p8).
#   APPLE_API_ISSUER_ID
#       App Store Connect issuer UUID.
#   APPLE_API_KEY_P8
#       Contents of the .p8 key file (or base64 of that file).
#   APPLE_API_KEY_PATH
#       Alternative to APPLE_API_KEY_P8: path to an existing .p8 file.
#   BEARCAD_REQUIRE_SIGN=1
#       Fail instead of ad-hoc signing when no Developer ID is available.
#   BEARCAD_REQUIRE_NOTARIZE=1
#       Fail when notarytool credentials are missing.
#   BEARCAD_FORCE_ADHOC=1
#       Ignore any Developer ID identity and ad-hoc sign (tests / local).
#   BEARCAD_KEYCHAIN_PATH / BEARCAD_KEYCHAIN_PASSWORD
#       Temp keychain used by import-keychain / delete-keychain.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ENTITLEMENTS="$ROOT/macos/BearCAD.entitlements"
QL_ENTITLEMENTS="$ROOT/macos/quicklook/BearCADQuickLook.entitlements"

default_keychain_path() {
  echo "${BEARCAD_KEYCHAIN_PATH:-${RUNNER_TEMP:-/tmp}/bearcad-signing.keychain-db}"
}

require_darwin() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "sign-macos.sh requires macOS" >&2
    exit 1
  fi
}

decode_base64() {
  # stdin → $1. openssl is on the Xcode CLT image; -A accepts a single line.
  openssl base64 -d -A >"$1"
}

nonempty() {
  [[ -n "${1//[$' \t\r\n']/}" ]]
}

resolve_identity() {
  if [[ "${BEARCAD_FORCE_ADHOC:-}" == "1" ]]; then
    return 0
  fi
  if nonempty "${APPLE_CODESIGN_IDENTITY:-}"; then
    printf '%s\n' "$APPLE_CODESIGN_IDENTITY"
    return 0
  fi
  security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application/ { print $2; exit }'
}

have_notary_creds() {
  nonempty "${APPLE_API_KEY_ID:-}" \
    && nonempty "${APPLE_API_ISSUER_ID:-}" \
    && { nonempty "${APPLE_API_KEY_P8:-}" || nonempty "${APPLE_API_KEY_PATH:-}"; }
}

write_api_key() {
  local dest="$1"
  if nonempty "${APPLE_API_KEY_PATH:-}"; then
    cp "$APPLE_API_KEY_PATH" "$dest"
    chmod 600 "$dest"
    return 0
  fi
  local body="${APPLE_API_KEY_P8:-}"
  if [[ "$body" != *"BEGIN PRIVATE KEY"* ]]; then
    printf '%s' "$body" | tr -d ' \t\r\n' | decode_base64 "$dest" || true
    if grep -q "BEGIN PRIVATE KEY" "$dest" 2>/dev/null; then
      chmod 600 "$dest"
      return 0
    fi
  fi
  printf '%s\n' "$body" >"$dest"
  chmod 600 "$dest"
}

unlock_signing_keychain() {
  local keychain password
  keychain="$(default_keychain_path)"
  password="${BEARCAD_KEYCHAIN_PASSWORD:-}"
  if [[ -f "$keychain" ]] && nonempty "$password"; then
    security unlock-keychain -p "$password" "$keychain" >/dev/null
  fi
}

import_p12() {
  local label="$1" b64="$2" password="$3" keychain="$4"
  if ! nonempty "$b64"; then
    return 1
  fi
  if ! nonempty "$password"; then
    echo "error: ${label} password is empty" >&2
    exit 1
  fi
  local p12
  p12="$(mktemp "${TMPDIR:-/tmp}/bearcad-cert.XXXXXX.p12")"
  printf '%s' "$b64" | tr -d ' \t\r\n' | decode_base64 "$p12"
  security import "$p12" -k "$keychain" -P "$password" -A \
    -T /usr/bin/codesign -T /usr/bin/security -T /usr/bin/productsign
  rm -f "$p12"
}

cmd_import_keychain() {
  require_darwin
  local keychain password
  keychain="$(default_keychain_path)"
  password="${BEARCAD_KEYCHAIN_PASSWORD:-}"
  if ! nonempty "$password"; then
    password="$(openssl rand -base64 24)"
  fi

  if ! nonempty "${APPLE_DEVELOPER_ID_APPLICATION_P12:-}"; then
    if [[ "${BEARCAD_REQUIRE_SIGN:-}" == "1" ]]; then
      echo "error: APPLE_DEVELOPER_ID_APPLICATION_P12 is not set" >&2
      exit 1
    fi
    echo "no APPLE_DEVELOPER_ID_APPLICATION_P12; skipping keychain import"
    return 0
  fi

  if [[ -f "$keychain" ]]; then
    security delete-keychain "$keychain" >/dev/null 2>&1 || rm -f "$keychain"
  fi
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  local existing
  existing="$(security list-keychains -d user | sed -E 's/^[[:space:]]*"|"$//g' | tr '\n' ' ')"
  # shellcheck disable=SC2086
  security list-keychains -d user -s "$keychain" $existing

  import_p12 "APPLE_DEVELOPER_ID_APPLICATION" \
    "$APPLE_DEVELOPER_ID_APPLICATION_P12" \
    "${APPLE_DEVELOPER_ID_APPLICATION_PASSWORD:-}" \
    "$keychain"

  if nonempty "${APPLE_DEVELOPER_ID_INSTALLER_P12:-}"; then
    import_p12 "APPLE_DEVELOPER_ID_INSTALLER" \
      "$APPLE_DEVELOPER_ID_INSTALLER_P12" \
      "${APPLE_DEVELOPER_ID_INSTALLER_PASSWORD:-}" \
      "$keychain"
  fi

  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null

  if [[ -n "${GITHUB_ENV:-}" ]]; then
    {
      echo "BEARCAD_KEYCHAIN_PATH=$keychain"
      echo "BEARCAD_KEYCHAIN_PASSWORD=$password"
    } >>"$GITHUB_ENV"
  fi
  echo "imported signing certificates into $keychain"
}

cmd_delete_keychain() {
  require_darwin
  local keychain
  keychain="$(default_keychain_path)"
  if [[ -f "$keychain" ]]; then
    security delete-keychain "$keychain" >/dev/null 2>&1 || rm -f "$keychain"
    echo "deleted $keychain"
  fi
}

adhoc_sign_app() {
  local app="$1"
  local appex="$app/Contents/PlugIns/BearCADQuickLook.appex"
  if [[ -d "$appex" ]]; then
    codesign --force --sign - --timestamp=none \
      --entitlements "$QL_ENTITLEMENTS" \
      "$appex"
  fi
  codesign --force --sign - --timestamp=none "$app"
  codesign --verify --deep --strict "$app"
  echo "ad-hoc signed $app"
}

developer_id_sign() {
  local target="$1" entitlements="${2:-}"
  local identity
  identity="$(resolve_identity)"
  local args=(--force --sign "$identity" --options runtime --timestamp)
  if nonempty "$entitlements"; then
    args+=(--entitlements "$entitlements")
  fi
  codesign "${args[@]}" "$target"
}

notarize_and_staple() {
  local artifact="$1"
  local work zip_or_dmg
  work="$(mktemp -d "${TMPDIR:-/tmp}/bearcad-notary.XXXXXX")"
  local key="$work/AuthKey.p8"
  write_api_key "$key"

  if [[ "$artifact" == *.dmg ]]; then
    zip_or_dmg="$artifact"
  else
    zip_or_dmg="$work/submit.zip"
    ditto -c -k --keepParent "$artifact" "$zip_or_dmg"
  fi

  echo "submitting $(basename "$artifact") to notarytool"
  xcrun notarytool submit "$zip_or_dmg" \
    --key "$key" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" \
    --wait \
    --timeout 30m
  rm -rf "$work"

  xcrun stapler staple "$artifact"
  xcrun stapler validate "$artifact"
  echo "stapled $artifact"
}

maybe_notarize() {
  local artifact="$1"
  if have_notary_creds; then
    notarize_and_staple "$artifact"
    return 0
  fi
  if [[ "${BEARCAD_REQUIRE_NOTARIZE:-}" == "1" ]]; then
    echo "error: BEARCAD_REQUIRE_NOTARIZE=1 but APPLE_API_KEY_ID / APPLE_API_ISSUER_ID / APPLE_API_KEY_P8 are not set" >&2
    exit 1
  fi
  echo "skipping notarization of $artifact (no App Store Connect API key)"
}

cmd_sign_app() {
  require_darwin
  local app="${1:-}"
  if [[ -z "$app" || ! -d "$app" ]]; then
    echo "usage: $0 sign-app <BearCAD.app>" >&2
    exit 1
  fi
  unlock_signing_keychain
  local identity
  identity="$(resolve_identity || true)"
  if ! nonempty "$identity"; then
    if [[ "${BEARCAD_REQUIRE_SIGN:-}" == "1" ]]; then
      echo "error: BEARCAD_REQUIRE_SIGN=1 but no Developer ID Application identity is available" >&2
      echo "       set APPLE_CODESIGN_IDENTITY or import a Developer ID Application .p12" >&2
      exit 1
    fi
    adhoc_sign_app "$app"
    return 0
  fi

  echo "signing $app as $identity"
  local appex="$app/Contents/PlugIns/BearCADQuickLook.appex"
  if [[ -d "$appex" ]]; then
    developer_id_sign "$appex" "$QL_ENTITLEMENTS"
  fi
  developer_id_sign "$app" "$APP_ENTITLEMENTS"
  codesign --verify --deep --strict "$app"
  maybe_notarize "$app"
}

cmd_sign_dmg() {
  require_darwin
  local dmg="${1:-}"
  if [[ -z "$dmg" || ! -f "$dmg" ]]; then
    echo "usage: $0 sign-dmg <bearcad.dmg>" >&2
    exit 1
  fi
  unlock_signing_keychain
  local identity
  identity="$(resolve_identity || true)"
  if nonempty "$identity"; then
    echo "signing $dmg as $identity"
    codesign --force --sign "$identity" --timestamp "$dmg"
    codesign --verify --strict "$dmg"
  elif [[ "${BEARCAD_REQUIRE_SIGN:-}" == "1" ]]; then
    echo "error: BEARCAD_REQUIRE_SIGN=1 but no Developer ID Application identity is available" >&2
    exit 1
  else
    echo "skipping Developer ID signature on $dmg (no identity)"
    return 0
  fi
  maybe_notarize "$dmg"
}

usage() {
  sed -n '2,40p' "$0"
  echo
  echo "usage: $0 <import-keychain|delete-keychain|sign-app|sign-dmg> [path]"
}

cmd="${1:-}"
case "$cmd" in
  import-keychain) cmd_import_keychain ;;
  delete-keychain) cmd_delete_keychain ;;
  sign-app) cmd_sign_app "${2:-}" ;;
  sign-dmg) cmd_sign_dmg "${2:-}" ;;
  -h|--help|help|"") usage ;;
  *)
    echo "unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac
