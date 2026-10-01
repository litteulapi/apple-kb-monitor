#!/usr/bin/env bash
# tests/keyd/check-config.sh - guards for the keyd example we ship (#246).
#
#   1. `keyd check` on keyd/apple-keyboard.conf (parser only: no daemon, no
#      /dev/uinput, no grab; skipped if keyd is not installed)
#   2. packaging: keyd is an optdepend, the config is NOT installed in
#      /etc/keyd (example under /usr/share/doc), and the .install scriptlet
#      never runs `keyd reload` nor (re)starts/stops keyd: `keyd reload`
#      crashes keyd 2.6.0 (dangling active_kbd, see docs/KEYD.md)
#   3. with --harness: tests/keyd/run-harness.sh (ASAN, keyd 2.6.0 sources)
#
# usage: tests/keyd/check-config.sh [--harness]
# exit: 0 ok, 1 failure
set -u
here=$(cd "$(dirname "$0")" && pwd)
top=$(cd "$here/../.." && pwd)
conf="$top/keyd/apple-keyboard.conf"
rc=0
fail() { echo "FAIL: $*"; rc=1; }

# 1. parser
if command -v keyd >/dev/null 2>&1; then
  out=$(keyd check "$conf" 2>&1); st=$?
  if [ $st -ne 0 ] || ! grep -q "No errors found" <<<"$out" || grep -qi "warn\|error:" <<<"$out"; then
    fail "keyd check $conf"; echo "$out" | tail -20
  else
    echo "ok    keyd check ($(keyd -v 2>/dev/null | head -1)): no errors, no warnings"
  fi
else
  echo "skip  keyd check (keyd not installed)"
fi

# 2. packaging
inst="$top/apple-kb-monitor.install"
# Only executable lines: echo'd instructions for the user may name keyd.
code=$(grep -v '^[[:space:]]*#' "$inst" | grep -v '^[[:space:]]*echo ')
if grep -Eq 'keyd[[:space:]]+reload|systemctl[^;|&]*(start|restart|try-restart|reload|stop|enable|disable)[^;|&]*keyd' <<<"$code"; then
  fail "$inst touches keyd:"; grep -En 'keyd[[:space:]]+reload|systemctl.*keyd' <<<"$code"
else
  echo "ok    .install never reloads/restarts keyd"
fi
pkgb="$top/PKGBUILD"
deps=$(sed -n '/^depends=(/,/)/p' "$pkgb")
grep -qw "'keyd'" <<<"$deps" && fail "keyd is still a hard dependency in PKGBUILD"
grep -q '"\$pkgdir/etc/keyd' "$pkgb" && fail "PKGBUILD still installs a config in /etc/keyd (active by default)"
grep -q "etc/keyd" <<<"$(sed -n '/^backup=(/,/)/p' "$pkgb")" && fail "PKGBUILD backup= still lists etc/keyd"
grep -q "^[[:space:]]*depends = keyd$" "$top/.SRCINFO" && fail ".SRCINFO still depends on keyd"
[ $rc -eq 0 ] && echo "ok    PKGBUILD/.SRCINFO: keyd optional, example in /usr/share/doc"

# 3. harness
if [ "${1:-}" = "--harness" ]; then
  bash "$here/run-harness.sh"; st=$?
  [ $st -eq 0 ] || [ $st -eq 77 ] || rc=1
fi
exit $rc
