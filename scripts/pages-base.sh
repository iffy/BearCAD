#!/usr/bin/env bash
# Print the GitHub Pages base URL (with trailing slash) for this repository,
# derived from GITHUB_REPOSITORY ("owner/repo"). Used by the docs CI (#1389) to
# fetch pre-built screenshots from the live site instead of rebuilding them on
# every push.
#
# Usage:
#   GITHUB_REPOSITORY="iffy/BearCAD" scripts/pages-base.sh   # -> https://iffy.github.io/BearCAD/
#   scripts/pages-base.sh      # reads GITHUB_REPOSITORY from the environment
set -euo pipefail

if [[ -z "${GITHUB_REPOSITORY:-}" ]]; then
  echo "error: GITHUB_REPOSITORY (owner/repo) is required" >&2
  exit 2
fi

owner="${GITHUB_REPOSITORY%/*}"
repo="${GITHUB_REPOSITORY#*/}"

# User/org sites are served at <owner>.github.io/ with no repo segment;
# project sites at <owner>.github.io/<repo>/.
pages="https://${owner}.github.io/"
if [[ "$owner" != "$repo" ]]; then
  pages+="${repo}/"
fi

printf '%s' "$pages"
