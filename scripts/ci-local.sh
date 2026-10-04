#!/usr/bin/env bash
# shellcheck disable=SC2329  # the s_* steps are called through step NAME ... -- s_x
# Local CI pipeline: one command, a JSON + text report, a reliable exit code.
#
#   scripts/ci-local.sh            full pipeline (static, build, tests, package, e2e)
#   scripts/ci-local.sh --fast     pre-push subset (the steps tagged `fast` in `--list`)
#   scripts/ci-local.sh --no-e2e   full pipeline without the Xvfb end-to-end tests
#   scripts/ci-local.sh --only a,b run only the named steps (see `--list`)
#
# Reports: scripts/out/<UTC stamp>/{report.json,report.txt,logs/*.log}, and
# scripts/out/latest -> the last run. Exit code: 0 = every required step
# passed, 1 = at least one required step failed, 64 = usage error.
# Never touches the keyboard nor the user's buses, never needs root, installs nothing.
#
# Environment: CARGO_TARGET_DIR (default: <repo>/apihub-app/target),
# AKM_CI_TEST_TIMEOUT (seconds, default 900): a test suite that does not end
# in time is a FAILURE (a test that hangs is a bug, not a slow machine).
# AKM_QA_ALLOW_DIR: directory of the *-allow.tsv lists of qa_checks.py (default
# scripts/; none = nothing tolerated). AKM_PUBLIC_EXCLUDE: file of paths
# the private and links steps leave out (one per line, "dir/" = directory).
set -uo pipefail
export LC_ALL=C LANG=C
# A relative CARGO_TARGET_DIR would be resolved from apihub-app/ by cargo.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then CARGO_TARGET_DIR=$(realpath -m "$CARGO_TARGET_DIR"); export CARGO_TARGET_DIR; fi

top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$top" || exit 1
mode=full e2e=1 only="" list=0
while [ $# -gt 0 ]; do
  case "$1" in
    --fast) mode=fast; e2e=0 ;;
    --no-e2e) e2e=0 ;;
    --only) only=",$2,"; shift ;;
    --list) list=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "usage: $0 [--fast|--no-e2e|--only a,b|--list]" >&2; exit 64 ;;
  esac
  shift
done

stamp=$(date -u +%Y%m%dT%H%M%SZ)
# AKM_CI_OUT moves the reports (and the package build copy) off the repository,
# e.g. onto a data partition; default: scripts/out in the repository.
out_root=${AKM_CI_OUT:-$top/scripts/out}
out="$out_root/$stamp"
mkdir -p "$out/logs"
ln -sfn "$stamp" "$out_root/latest"
: > "$out/steps.tsv"
test_timeout=${AKM_CI_TEST_TIMEOUT:-900}
commit=$(git rev-parse --short HEAD 2>/dev/null || echo "?")
dirty=$(git status --porcelain 2>/dev/null | grep -c . || true)
printf '{"mode":"%s","commit":"%s","dirty_files":%s,"started":"%s","host":"%s"}\n' \
  "$mode" "$commit" "${dirty:-0}" "$stamp" "$(uname -n)" > "$out/meta.json"

have() { command -v "$1" >/dev/null 2>&1; }

# No step may reach the user's buses (zbus falls back to $XDG_RUNTIME_DIR/bus):
# a private runtime dir and bus addresses where no bus listens.
ci_run=$(mktemp -d "${TMPDIR:-/tmp}/akm-ci-run.XXXXXX") || exit 1
trap 'rm -rf "$ci_run"' EXIT
export XDG_RUNTIME_DIR="$ci_run"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$ci_run/bus" DBUS_SYSTEM_BUS_ADDRESS="unix:path=$ci_run/system-bus"
unset DBUS_STARTER_ADDRESS DBUS_STARTER_BUS_TYPE

# step NAME REQUIRED(1|0) TAGS -- command...
#   TAGS: "fast" = also part of --fast (pre-push). Status: pass/fail/skip.
#   A command exiting 77 means "skipped" (tool missing), its last line says why.
step() {
  local name=$1 required=$2 tags=$3; shift 4
  if [ "$list" = 1 ]; then printf '%-16s required=%s tags=%s\n' "$name" "$required" "$tags"; return 0; fi
  if [ -n "$only" ] && [[ $only != *",$name,"* ]]; then return 0; fi
  if [ -z "$only" ] && [ "$mode" = fast ] && [[ $tags != *fast* ]]; then return 0; fi
  local log="$out/logs/$name.log" t0 t1 rc status summary
  printf '==> %-16s ' "$name"
  t0=$(date +%s.%N)
  ( "$@" ) > "$log" 2>&1 </dev/null
  rc=$?
  t1=$(date +%s.%N)
  case $rc in
    0) status=pass ;;
    77) status=skip ;;
    *) status=fail ;;
  esac
  summary=$(grep -v '^[[:space:]]*$' "$log" | tail -1 | tr '\t' ' ' | cut -c1-160)
  [ $status = fail ] && [ $rc = 124 ] && summary="TIMEOUT (hang?) after limit: $summary"
  local secs
  secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.1f", b - a }')
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$status" "$secs" "$required" "logs/$name.log" "$summary" >> "$out/steps.tsv"
  printf '%s (%ss)%s\n' "$status" "$secs" "$([ $status = fail ] && echo " -> $log")"
  if [ $status = fail ]; then grep -v '^[[:space:]]*$' "$log" | tail -15 | sed 's/^/      /'; fi
}

# ── step implementations (each runs in a subshell, cwd = repo top) ─────────

s_versions() {
  local cargo_v pkg_v src_v log_v
  cargo_v=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' apihub-app/Cargo.toml | head -1)
  pkg_v=$(sed -n 's/^pkgver=//p' PKGBUILD)
  src_v=$(sed -n 's/^[[:space:]]*pkgver = //p' .SRCINFO)
  log_v=$(sed -n 's/^## \[\([0-9][^]]*\)\].*/\1/p' CHANGELOG.md | head -1)
  local pr_b pr_s
  pr_b=$(sed -n 's/^pkgrel=//p' PKGBUILD); pr_s=$(sed -n 's/^[[:space:]]*pkgrel = //p' .SRCINFO)
  echo "cargo=$cargo_v pkgbuild=$pkg_v-$pr_b srcinfo=$src_v-$pr_s changelog=$log_v"
  [ -n "$cargo_v" ] && [ "$cargo_v" = "$pkg_v" ] && [ "$cargo_v" = "$src_v" ] && [ "$cargo_v" = "$log_v" ] \
    && [ "$pr_b" = "$pr_s" ] || { echo "version mismatch"; return 1; }
  # Every file the PKGBUILD installs from $startdir must exist in the tree.
  local miss=0 f
  # shellcheck disable=SC2016  # literal $startdir searched in the PKGBUILD
  for f in $(grep -oE '"\$startdir/[^"*]+"' PKGBUILD | tr -d '"' | sed 's|\$startdir/||' | sort -u); do
    case $f in apihub-app/target/*|rssi-helper) continue ;; esac
    [ -e "$f" ] || { echo "PKGBUILD installs a missing file: $f"; miss=1; }
  done
  [ "$miss" = 0 ] && echo "versions consistent: $cargo_v"
  return "$miss"
}

s_fmt() { (cd apihub-app && cargo fmt --all --check 2>&1 | tail -40; exit "${PIPESTATUS[0]}"); }

s_clippy() { (cd apihub-app && timeout 1200 cargo clippy --locked --workspace --all-targets -- -D warnings 2>&1 | tail -60; exit "${PIPESTATUS[0]}"); }

s_test() {
  # --kill-after: a hung test binary is killed and reported as TIMEOUT.
  # Full output: logs/test-full.log; this log keeps results + each failure.
  local full="$out/logs/test-full.log" rc
  (cd apihub-app && AKM_REQUIRE_DBUS=1 timeout --kill-after=10 "$test_timeout" cargo test --locked --workspace > "$full" 2>&1)
  rc=$?
  grep -E '^test result|^error|has been running for over' "$full" | tail -40
  grep -n -A12 -E "^---- |panicked at" "$full" | head -120
  [ "$rc" = 124 ] || [ "$rc" = 137 ] && echo "TIMEOUT: cargo test did not finish in ${test_timeout}s (a test hangs; see logs/test-full.log for the last test started)"
  [ "$rc" = 0 ] && echo "all tests passed: $(grep -E '^test result' "$full" | awk '{s+=$4} END {print s}') test(s)"
  return "$rc"
}


s_deny() {
  have cargo-deny || { echo "cargo-deny not installed"; return 77; }
  (cd apihub-app && cargo deny --log-level error check advisories bans sources 2>&1 | tail -30; exit "${PIPESTATUS[0]}")
}

s_audit() {
  have cargo-audit || { echo "cargo-audit not installed"; return 77; }
  # --no-fetch when offline: the local advisory DB is used as is.
  local ign
  ign=$(sed -n 's/^[[:space:]]*{ *id *= *"\(RUSTSEC-[0-9-]*\)".*/--ignore \1/p; s/^[[:space:]]*"\(RUSTSEC-[0-9-]*\)".*/--ignore \1/p' apihub-app/deny.toml | tr '\n' ' ')
  # shellcheck disable=SC2086
  (cd apihub-app && { timeout 120 cargo audit -q $ign 2>&1 || timeout 60 cargo audit -q --no-fetch $ign 2>&1; } | tail -30; exit "${PIPESTATUS[0]}")
}

s_udev() {
  have udevadm || { echo "udevadm missing"; return 77; }
  local f rc=0
  for f in udev/*.rules; do udevadm verify --no-style "$f" || rc=1; done
  [ "$rc" = 0 ] && echo "udev rules valid"
  return "$rc"
}

s_qml() {
  # Qt 6 qmllint (the one in PATH may be Qt 5's, which rejects these options)
  local q=${QMLLINT:-/usr/lib/qt6/bin/qmllint} f rc=0 out
  [ -x "$q" ] || { echo "qmllint missing: $q"; return 77; }
  for f in plasma/com.agenceapi.devicehub/contents/ui/*.qml plasma/tests/*.qml; do
    # Plasma imports are not resolvable outside a Plasma session: imports and
    # unqualified access are not checked, any other warning (syntax included) fails.
    out=$("$q" --unqualified disable --import disable --unused-imports disable "$f" 2>&1) \
      || { echo "$out" | head -20; rc=1; }
  done
  # The KCM pages: kcm/lint/qmllint.sh fails on any warning (syntax
  # included) except the run-time context properties.
  local krc=0
  bash kcm/lint/qmllint.sh 2>&1 | tail -20; krc=${PIPESTATUS[0]}
  [ "$krc" = 0 ] || rc=1
  [ "$rc" = 0 ] && echo "qml: no syntax error (plasma + kcm)"
  return "$rc"
}

s_kcm() {
  # The KCM plugin is otherwise only compiled by makepkg: configure,
  # build and run its tests when cmake + ECM are present.
  have cmake || { echo "cmake missing"; return 77; }
  [ -d /usr/share/ECM ] || { echo "extra-cmake-modules (ECM) missing"; return 77; }
  local b="${AKM_CI_KCM_BUILD:-${CARGO_TARGET_DIR:-$top/apihub-app/target}-kcm}" rc
  mkdir -p "$b"
  cmake -S kcm -B "$b" -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON > "$out/logs/kcm-cmake.log" 2>&1 \
    || { tail -30 "$out/logs/kcm-cmake.log"; return 1; }
  timeout 1200 cmake --build "$b" -j"$(nproc)" > "$out/logs/kcm-build.log" 2>&1 \
    || { grep -E 'error|Error' "$out/logs/kcm-build.log" | head -30; return 1; }
  (cd apihub-app && cargo build -q --locked -p apple-kb-monitord --features testbus) || return 1
  rm -f "$b/Testing/Temporary/LastTest.log"
  (cd "$b" && AKM_DAEMON="${CARGO_TARGET_DIR:-$top/apihub-app/target}/debug/apple-kb-monitord" timeout 300 ctest --output-on-failure 2>&1 | tail -15; exit "${PIPESTATUS[0]}"); rc=$?
  # the full output of every test, for the CI artifact (the console keeps only the tail)
  cp "$b/Testing/Temporary/LastTest.log" "$out/logs/kcm-ctest.log" 2>/dev/null
  [ "$rc" = 0 ] || return 1
  echo "kcm: plugin builds, ctest passes"
}

s_shell() {
  local f rc=0 n=0
  while IFS= read -r f; do
    n=$((n + 1))
    case $(head -1 "$f") in *bash*) bash -n "$f" || rc=1 ;; *) sh -n "$f" || rc=1 ;; esac
  done < <( { git ls-files '*.sh' ':(glob)**/githooks/*' apple-kb-monitor.install; git ls-files --others --exclude-standard '*.sh' ':(glob)**/githooks/*'; } | sort -u | while read -r x; do [ -f "$x" ] && echo "$x"; done)
  if have shellcheck; then
    # shellcheck disable=SC2046
    shellcheck -S warning $(git ls-files '*.sh' ':(glob)**/githooks/*' apple-kb-monitor.install; git ls-files --others --exclude-standard '*.sh' ':(glob)**/githooks/*') || rc=1
    echo "bash -n + shellcheck on $n file(s)"
  else
    echo "bash -n on $n file(s) (shellcheck not installed: syntax only)"
  fi
  bash tests/check-install-sh.sh || rc=1
  return "$rc"
}

s_c() { gcc -Wall -Wextra -Werror -o "$out/rssi-helper" rssi-helper.c && echo "rssi-helper builds with -Werror"; }

s_security() { sh tests/check-security-files.sh && sh tests/check-trademarks.sh && sh tests/check-data-files.sh && sh tests/check-sleep-units.sh && sh tests/check-po.sh && bash tests/check-po-stable.sh && python3 plasma/tests/check_plaintext.py && bash tests/check-install-scriptlet.sh && bash tests/keyd/check-config.sh && bash tests/check-ci-workflow.sh && echo "security files + data files + plain text + install scriptlet + keyd example OK"; }

s_secrets() { python3 scripts/qa_checks.py secrets; }
s_claims() { python3 scripts/qa_checks.py claims; }
s_redaction() { python3 scripts/qa_checks.py redaction; }
s_private() {
  local a b
  python3 scripts/qa_checks.py private; a=$?
  python3 scripts/qa_checks.py tracker | tail -15; b=${PIPESTATUS[0]}
  [ "$a" = 0 ] && [ "$b" = 0 ]
}
s_links() { python3 scripts/qa_checks.py links; }

s_package() {
  have makepkg || { echo "makepkg missing"; return 77; }
  # Build in a copy of the working tree: the repository stays untouched.
  local cp="$out/pkgbuild" pkg
  mkdir -p "$cp"
  { git ls-files -z --cached; git ls-files -z --others --exclude-standard; } \
    | (cd "$top" && xargs -0 -I{} sh -c '[ -f "{}" ] && echo "{}"') | grep -v '^scripts/out/' \
    | tar -C "$top" -cf - -T - | tar -C "$cp" -xf -
  # AKM_ALLOW_DIRTY=1: this copy of the working tree is deliberately not a
  # commit (the PKGBUILD refuses such a tree otherwise).
  # Reuse a target dir between runs (the PKGBUILD builds into apihub-app/target).
  local cache="${AKM_CI_PKG_TARGET:-${CARGO_TARGET_DIR:-$top/apihub-app/target}-pkg}"
  mkdir -p "$cache" && ln -sfn "$cache" "$cp/apihub-app/target"
  (cd "$cp" && AKM_ALLOW_DIRTY=1 PKGDEST="$cp" SRCDEST="$cp" BUILDDIR="$cp/build" timeout 3600 makepkg -f --nodeps --noconfirm --nosign >"$out/logs/makepkg-full.log" 2>&1) \
    || { tail -30 "$out/logs/makepkg-full.log"; return 1; }
  pkg=$(printf '%s\n' "$cp"/*.pkg.tar.* | head -1)
  echo "package: $(basename "$pkg") ($(du -h "$pkg" | cut -f1))"
  python3 scripts/qa_checks.py package "$pkg"
}

s_plasma() {
  [ -x plasma/tests/run-widget-tests.sh ] || [ -f plasma/tests/run-widget-tests.sh ] || { echo "no widget tests"; return 77; }
  have qml6 && have dbus-run-session && python3 -c 'import dbus, gi' 2>/dev/null \
    || { echo "qml6 / dbus-run-session / python3-dbus + gi missing"; return 77; }
  timeout 300 bash plasma/tests/run-widget-tests.sh 2>&1 | tail -20; return "${PIPESTATUS[0]}"
}

# Fixture tests of the reverse-engineering tools (no keyboard needed).
s_re() {
  python3 -c 'import pytest' 2>/dev/null || { echo "python-pytest missing"; return 77; }
  PYTHONDONTWRITEBYTECODE=1 timeout 300 python3 -m pytest -q -p no:cacheprovider tests/live/re 2>&1 | tail -15; return "${PIPESTATUS[0]}"
}

s_selftest_unit() {
  # The selfcheck units must parse (systemd-analyze verify needs the binary path: use a stub).
  have systemd-analyze || { echo "systemd-analyze missing"; return 77; }
  local u rc=0
  for u in systemd/*.service systemd/*.timer; do
    systemd-analyze --user verify --man=no "$u" 2>&1 | grep -vE 'Command .* is not executable|No such file|Failed to prepare filename|not found|Unit .* not found' | grep . && rc=1
  done
  [ "$rc" = 0 ] && echo "systemd units parse"
  return "$rc"
}

# Widget in a throwaway plasmashell, then the KCM in kcmshell6 (tests/e2e/kcm.py); 77 = tools missing.
s_e2e() {
  local w k
  timeout --kill-after=20 1500 bash tests/e2e/run.sh --out "$out/e2e"; w=$?
  timeout --kill-after=20 1500 python3 tests/e2e/kcm.py --out "$out/e2e-kcm" 2>&1 | tail -15; k=${PIPESTATUS[0]}
  [ "$w" != 0 ] && [ "$w" != 77 ] && return "$w"
  [ "$k" != 0 ] && [ "$k" != 77 ] && return "$k"
  { [ "$w" = 77 ] || [ "$k" = 77 ]; } && { echo "e2e partly skipped (widget=$w kcm=$k)"; return 77; }
  return 0
}

# ── pipeline ────────────────────────────────────────────────────────────────

step versions   1 fast -- s_versions
step secrets    1 fast -- s_secrets
step fmt        1 fast -- s_fmt
step clippy     1 fast -- s_clippy
step test       1 fast -- s_test
step claims     1 ""   -- s_claims
step redaction  1 ""   -- s_redaction
step private    1 ""   -- s_private
step links      1 ""   -- s_links
step deny       1 ""   -- s_deny
step audit      0 ""   -- s_audit
step udev       1 ""   -- s_udev
step qml        1 ""   -- s_qml
step kcm        1 fast -- s_kcm
step shell      1 fast -- s_shell
step c          1 ""   -- s_c
step security   1 ""   -- s_security
step units      1 ""   -- s_selftest_unit
step plasma     1 ""   -- s_plasma
step re         1 ""   -- s_re
step package    1 ""   -- s_package
if [ $e2e = 1 ] || [[ $only == *",e2e,"* ]]; then step e2e 1 "" -- s_e2e; fi
[ "$list" = 1 ] && exit 0

python3 scripts/qa_checks.py report "$out" > /dev/null
rc=$?
echo
tail -1 "$out/report.txt"
echo "report: $out/report.txt  (json: $out/report.json)"
exit $rc
