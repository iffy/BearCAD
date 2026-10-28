#!/usr/bin/env bash
# Fetch the solvespace submodule plus the mimalloc extlib libslvs needs. Eigen is
# vendored in third_party/eigen: its upstream (gitlab.com) refuses clones under load.
#
# Retries with backoff in case GitHub hiccups.
set -euo pipefail

cd "$(dirname "$0")/.."

attempts=6
delay=15
for ((i = 1; i <= attempts; i++)); do
  if git submodule update --init --depth 1 third_party/solvespace &&
    git -C third_party/solvespace submodule update --init --depth 1 extlib/mimalloc; then
    exit 0
  fi
  if ((i < attempts)); then
    echo "Submodule fetch failed (attempt $i/$attempts); retrying in ${delay}s" >&2
    sleep "$delay"
    delay=$((delay * 2))
  fi
done
echo "error: could not fetch solvespace submodules after $attempts attempts" >&2
exit 1
