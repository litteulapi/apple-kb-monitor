#!/bin/sh
# Static guard of the sleep / wake system units. No hardware, no root.
# The helper they run (akm-hid-control) must be able to read the daemon's
# circuit-breaker state under /run/user/<uid>: ProtectHome=yes hides that
# directory and the breaker was silently ignored (the helper reported
# "NoState: emission allowed"), measured on 2026-10-02.
set -eu
top=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }
for u in apple-kb-monitor-suspend.service apple-kb-monitor-resume.service; do
  f="$top/systemd/$u"
  [ -f "$f" ] || fail "$u missing"
  n=$(grep -c -E '^[[:space:]]*ProtectHome=' "$f" || true)
  [ "$n" = 1 ] || fail "$u: exactly one ProtectHome= expected, found $n"
  grep -qx 'ProtectHome=read-only' "$f" \
    || fail "$u: ProtectHome must be read-only (yes/tmpfs hide /run/user and the breaker state)"
  for d in InaccessiblePaths TemporaryFileSystem; do
    if grep -E "^[[:space:]]*$d=.*(/run/user|/run([[:space:]]|\$))" "$f" >/dev/null; then
      fail "$u: $d hides /run/user"
    fi
  done
  grep -q 'CAP_DAC_READ_SEARCH' "$f" || fail "$u: CAP_DAC_READ_SEARCH needed to read the user's 0700 runtime directory"
done
# SUSPEND left the keyboard dead on the real hardware (2026-10-02): the shipped
# configuration must keep the feature off.
grep -qx 'enabled = false' "$top/systemd/hid-suspend.conf" \
  || fail "systemd/hid-suspend.conf must ship 'enabled = false' (SUSPEND measured harmful on 2026-10-02)"
echo "sleep units: breaker state readable from the sandbox, SUSPEND off by default"
