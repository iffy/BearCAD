#!/usr/bin/env bash
# Fetch the latest published (non-draft, non-prerelease) release's WASM web app
# into web/dist/. The web app is built once per release by the release CI (see
# .github/workflows/ci.yml) and attached to the release as a bearcad-web.zip asset;
# the website CI downloads it here instead of rebuilding the ~30-min wasm app on
# every docs push (#1387).
#
# Usage:
#   scripts/fetch-web-release.sh          # needs gh + GITHUB_REPOSITORY (or --repo)
#   scripts/fetch-web-release.sh --repo owner/repo
#
# Exits 0 after populating web/dist/ from the newest release that carries the asset;
# exits 2 when no published release has one yet (caller may fall back to a local build).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REPO=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      REPO="${2:?--repo requires owner/repo}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,15p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "usage: $0 [--repo owner/repo]" >&2
      exit 1
      ;;
  esac
done
if [[ -z "$REPO" ]]; then
  REPO="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set (or pass --repo)}"
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh CLI is required" >&2
  exit 1
fi

ASSET="bearcad-web.zip"
OUT="web/dist"

# Newest first; keep only published (non-draft, non-prerelease) releases and pick
# the first one that actually carries the web asset (older releases predate it).
tag=""
while IFS= read -r t; do
  if gh release view "$t" --repo "$REPO" --json assets \
      --jq '.assets[].name' 2>/dev/null | grep -qx "$ASSET"; then
    tag="$t"
    break
  fi
done < <(gh release list --repo "$REPO" --limit 50 \
          --json tagName,isDraft,isPrerelease \
          --jq '.[] | select((.isDraft|not) and (.isPrerelease|not)) | .tagName')

if [[ -z "$tag" ]]; then
  echo "error: no published non-draft release carries ${ASSET}" >&2
  echo "       publish a release built after the web-asset change lands (#1387), or" >&2
  echo "       fall back to a local build (scripts/build-web.sh)." >&2
  exit 2
fi

echo "==> fetching web app from release ${tag} (${ASSET})"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gh release download "$tag" --repo "$REPO" --pattern "$ASSET" --clobber --dir "$tmp"

rm -rf "$OUT"
mkdir -p "$OUT"
unzip -q -o "$tmp/$ASSET" -d "$OUT"

echo "Fetched web app (${tag}) into $OUT:"
ls -la "$OUT"
