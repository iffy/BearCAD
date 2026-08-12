#!/usr/bin/env bash
# Changer-driven release changelog (#1328).
#
# Usage:
#   scripts/release-changelog.sh next-version
#   scripts/release-changelog.sh notes
#   scripts/release-changelog.sh full [outfile]
#   scripts/release-changelog.sh bump [version]
#   scripts/release-changelog.sh apply-published <version> <release-sha>
#
# Requires `changer` on PATH. next-version / notes / full are dry-run (no files
# change). bump and apply-published write CHANGELOG.md and consume snippets.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

need_changer() {
  if ! command -v changer >/dev/null 2>&1; then
    echo "changer is required (nimble install changer)" >&2
    exit 1
  fi
}

# changer prints the version with a trailing newline.
changer_version() {
  changer "$1" | tr -d '[:space:]'
}

cmd="${1:-}"
shift || true

case "$cmd" in
  next-version)
    need_changer
    changer_version next-version
    echo
    ;;
  notes)
    need_changer
    # stdout is the new CHANGELOG section; status lines go to stderr.
    changer bump -n
    ;;
  full)
    need_changer
    dest="${1:-}"
    # Redirect, don't capture: `$(...)` strips trailing newlines changer leaves
    # between the new section and the existing CHANGELOG.
    if [[ -n "$dest" ]]; then
      changer bump -n >"$dest"
      cat CHANGELOG.md >>"$dest"
    else
      changer bump -n
      cat CHANGELOG.md
    fi
    ;;
  bump)
    need_changer
    if [[ -n "${1:-}" ]]; then
      changer bump "$1"
    else
      changer bump
    fi
    ;;
  apply-published)
    need_changer
    version="${1:?apply-published requires <version>}"
    sha="${2:?apply-published requires <release-sha>}"
    current="$(changer_version current-version)"
    if [[ "$current" == "$version" ]]; then
      echo "CHANGELOG already at v${version}; skipping bump"
      exit 0
    fi
    held="$(mktemp -d)"
    released_list="$(mktemp)"
    trap 'rm -rf "$held" "$released_list"' EXIT
    git ls-tree -r --name-only "$sha" -- changes \
      | grep -E '^changes/(fix|new|break|other)-' >"$released_list" || true
    shopt -s nullglob
    for f in changes/{fix,new,break,other}-*.md; do
      if ! grep -qx "$f" "$released_list"; then
        mv "$f" "$held/"
      fi
    done
    while IFS= read -r rel || [[ -n "${rel:-}" ]]; do
      [[ -z "$rel" ]] && continue
      if [[ ! -f "$rel" ]]; then
        git show "$sha:$rel" >"$rel"
      fi
    done <"$released_list"
    changer bump "$version"
    if [[ -n "$(ls -A "$held" 2>/dev/null || true)" ]]; then
      mv "$held"/* changes/
    fi
    ;;
  -h|--help|help)
    sed -n '2,14p' "$0" | sed 's/^# \?//'
    ;;
  *)
    echo "usage: $0 next-version|notes|full [outfile]|bump [version]|apply-published <version> <sha>" >&2
    exit 1
    ;;
esac
