#!/usr/bin/env bash
# Next release build number: YYMMDD-### (zero-padded sequence per UTC day).
#
# Usage:
#   scripts/next-build-number.sh [--date YYMMDD] [--from-github]
#   printf '%s\n' tags... | scripts/next-build-number.sh [--date YYMMDD]
#
# Existing release tags are read from stdin (one per line). With --from-github
# (or a TTY stdin and GITHUB_REPOSITORY set), tags come from `gh release list`
# — including drafts, which often have no git tag ref yet.
#
# Prints e.g. 260812-001
set -euo pipefail

DATE=""
FROM_GITHUB=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --date)
      DATE="${2:?--date requires YYMMDD}"
      shift 2
      ;;
    --from-github)
      FROM_GITHUB=1
      shift
      ;;
    -h|--help)
      sed -n '2,12p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "usage: $0 [--date YYMMDD] [--from-github]" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$DATE" ]]; then
  DATE="$(date -u +%y%m%d)"
fi

if ! [[ "$DATE" =~ ^[0-9]{6}$ ]]; then
  echo "date must be YYMMDD (got: $DATE)" >&2
  exit 1
fi

read_tags() {
  if ((FROM_GITHUB)); then
    local repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set with --from-github}"
    if ! command -v gh >/dev/null 2>&1; then
      echo "gh CLI is required for --from-github" >&2
      exit 1
    fi
    # Draft releases may lack git tag refs (softprops/action-gh-release); list
    # release tagNames so the per-day sequence still advances.
    gh release list --repo "$repo" --limit 200 \
      --json tagName --jq '.[].tagName'
    return
  fi
  if [[ -t 0 ]]; then
    if [[ -n "${GITHUB_REPOSITORY:-}" ]] && command -v gh >/dev/null 2>&1; then
      FROM_GITHUB=1
      read_tags
      return
    fi
    return
  fi
  cat
}

# Temp file instead of process substitution: Git Bash on Windows often lacks
# working /dev/fd, which made `done < <(read_tags)` exit 1 with empty stderr.
tags_file="$(mktemp)"
trap 'rm -f "$tags_file"' EXIT
read_tags >"$tags_file"

max=0
while IFS= read -r tag || [[ -n "${tag:-}" ]]; do
  [[ -z "$tag" ]] && continue
  # Match ...-build.YYMMDD-NNN (suffix or followed by -gSHA describe noise).
  if [[ "$tag" =~ build\.${DATE}-([0-9]+) ]]; then
    # 10# forces base-10 (leading zeros).
    n=$((10#${BASH_REMATCH[1]}))
    if ((n > max)); then
      max=$n
    fi
  fi
done <"$tags_file"

next=$((max + 1))
printf '%s-%03d\n' "$DATE" "$next"
