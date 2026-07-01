#!/usr/bin/env bash
# Keep only the newest draft GitHub releases; delete older drafts and their tags.
# Usage: scripts/prune-draft-releases.sh [keep_count]
set -euo pipefail

KEEP="${1:-2}"
REPO="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required" >&2
  exit 1
fi

mapfile -t DRAFTS < <(
  gh release list --repo "$REPO" --limit 100 \
    --json tagName,isDraft,createdAt \
    --jq '[.[] | select(.isDraft)] | sort_by(.createdAt) | reverse | .[].tagName'
)

if ((${#DRAFTS[@]} <= KEEP)); then
  echo "Keeping all ${#DRAFTS[@]} draft release(s) (limit ${KEEP})"
  exit 0
fi

for ((i = KEEP; i < ${#DRAFTS[@]}; i++)); do
  tag="${DRAFTS[$i]}"
  echo "Deleting draft release ${tag}"
  gh release delete "$tag" --repo "$REPO" --yes
  # Draft releases may exist without a matching tag ref; ignore missing refs.
  gh api -X DELETE "repos/${REPO}/git/refs/tags/${tag}" 2>/dev/null || true
done

echo "Kept ${KEEP} newest draft(s); deleted $((${#DRAFTS[@]} - KEEP)) older draft(s)"