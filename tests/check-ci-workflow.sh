#!/usr/bin/env bash
# The public CI (.github/workflows/ci.yml) runs every required step of
# scripts/ci-local.sh (but package, built by package.yml, and e2e) and fails
# when any of its reports has a skipped step.
set -euo pipefail
cd "$(dirname "$0")/.."
wf=.github/workflows/ci.yml
rc=0
run=$(grep -o "ci-local.sh --only [a-z,]*" "$wf" | sed 's/.* //' | tr ',' '\n' | sort -u)
for s in $(sed -n 's/^step \([a-z0-9]*\) *1 .*/\1/p' scripts/ci-local.sh); do
  case $s in package | e2e) continue ;; esac
  grep -qx "$s" <<<"$run" || { echo "FAIL check-ci-workflow: required step '$s' not run by $wf" >&2; rc=1; }
done
grep -qF "! grep -E '^\\\\[skip\\\\]' scripts/out/2*/report.txt" "$wf" \
  || { echo "FAIL check-ci-workflow: $wf does not fail on a skipped step in every report" >&2; rc=1; }
# No step may reach the user's session daemon through $XDG_RUNTIME_DIR/bus.
grep -qx 'export XDG_RUNTIME_DIR="$ci_run"' scripts/ci-local.sh && grep -q '^export DBUS_SESSION_BUS_ADDRESS="unix:path=$ci_run/' scripts/ci-local.sh \
  || { echo "FAIL check-ci-workflow: scripts/ci-local.sh does not run its steps on a private XDG_RUNTIME_DIR and bus" >&2; rc=1; }
[ "$rc" = 0 ] && echo "public CI runs every required step and rejects skips; ci-local is off the user's buses"
exit "$rc"
