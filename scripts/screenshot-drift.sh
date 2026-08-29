#!/usr/bin/env bash
# How far the published doc screenshots lag master, and whether that is too far (#1836).
#
# The deployed site publishes `img/screenshots/screenshots-commit.txt`: the SHA the shots
# were captured at. A nightly that fails leaves it in place, so the drift keeps growing
# while every push-triggered run stays green. This turns the marker into numbers the
# workflow can act on.
#
# Inputs (env, all optional — the workflow sets the first two from the fetched marker):
#   DRIFT_MARKER          SHA the screenshots were captured at ("" = never published)
#   DRIFT_MARKER_TIME     commit time of that SHA, unix seconds (overrides git lookup)
#   DRIFT_BEHIND          commits from the marker to HEAD (overrides git lookup)
#   DRIFT_MAX_DAYS        stale beyond this many days behind (default 2)
#   DRIFT_MAX_BEHIND      stale beyond this many commits behind (default 25)
#
# Prints `key=value` lines (also appended to $GITHUB_OUTPUT when set):
#   stale=true|false  behind=N  days=N  marker=SHA  reason=...
set -uo pipefail

MAX_DAYS="${DRIFT_MAX_DAYS:-2}"
MAX_BEHIND="${DRIFT_MAX_BEHIND:-25}"
marker="${DRIFT_MARKER:-}"

behind="${DRIFT_BEHIND:-}"
marker_time="${DRIFT_MARKER_TIME:-}"

if [[ -z "$marker_time" && -n "$marker" ]]; then
  marker_time="$(git log -1 --format=%ct "$marker" 2>/dev/null || true)"
fi
if [[ -z "$behind" && -n "$marker" ]]; then
  behind="$(git rev-list --count "$marker"..HEAD 2>/dev/null || true)"
fi

reason=""
if [[ -z "$marker" && -z "$marker_time" && -z "$behind" ]]; then
  # Nothing has ever been published: there is no drift to measure, only a missing set.
  stale=true
  days=0
  behind=0
  reason="no screenshots have ever been published"
else
  behind="${behind:-0}"
  days=0
  if [[ -n "$marker_time" ]]; then
    days=$(( ( $(date +%s) - marker_time ) / 86400 ))
    (( days < 0 )) && days=0
  fi
  stale=false
  if (( days > MAX_DAYS )); then
    stale=true
    reason="screenshots were captured $days days ago (limit $MAX_DAYS)"
  elif (( behind > MAX_BEHIND )); then
    stale=true
    reason="screenshots are $behind commits behind master (limit $MAX_BEHIND)"
  fi
fi

emit() {
  printf '%s\n' "$1"
  [[ -n "${GITHUB_OUTPUT:-}" ]] && printf '%s\n' "$1" >> "$GITHUB_OUTPUT"
  return 0
}
emit "stale=$stale"
emit "behind=$behind"
emit "days=$days"
emit "marker=$marker"
emit "reason=$reason"
