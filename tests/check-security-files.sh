#!/bin/sh
# Static guards of the security audit 2 fixes (#203, #209, #211). No hardware,
# no root, nothing installed. Run from anywhere: sh tests/check-security-files.sh
set -eu
top=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }
unit="$top/systemd/apple-kb-monitord.service"

# #211: the daemon starts pkexec (setuid) and rssi-helper (file capability).
# These directives set NoNewPrivs or create a user namespace in a user unit and
# silently disable both: they must stay out (comment lines are ignored).
for d in NoNewPrivileges ProtectSystem ProtectHome PrivateTmp PrivateUsers \
         SystemCallFilter RestrictAddressFamilies MemoryDenyWriteExecute \
         LockPersonality RestrictRealtime RestrictSUIDSGID SystemCallArchitectures \
         RestrictNamespaces ProtectKernelTunables ProtectControlGroups; do
  if grep -E "^[[:space:]]*$d=" "$unit" >/dev/null; then
    fail "$d= in the user unit breaks pkexec / rssi-helper (see the comment in the unit)"
  fi
done
for d in UMask=0077 LimitCORE=0 KeyringMode=private; do
  grep -qx "$d" "$unit" || fail "$d missing from the unit"
done

# #203: no retained authorization (the subject is the long-lived daemon).
if grep -E "<allow_[a-z]+>auth_admin_keep<" "$top/polkit/com.agenceapi.AppleKbMonitor.policy" >/dev/null; then
  fail "polkit action must not use auth_admin_keep"
fi

# #209: rssi-helper restricted to group akm, capability applied last.
grep -q "sysusers.d/apple-kb-monitor.conf" "$top/PKGBUILD" || fail "PKGBUILD does not ship the akm group"
grep -qx "g akm - -" "$top/sysusers/apple-kb-monitor.conf" || fail "sysusers entry"
inst="$top/apple-kb-monitor.install"
c=$(grep -n 'chgrp akm' "$inst" | head -1 | cut -d: -f1)
s=$(grep -n 'setcap cap_net_admin+ep "\$h"' "$inst" | head -1 | cut -d: -f1)
[ -n "$c" ] && [ -n "$s" ] && [ "$c" -lt "$s" ] || fail "chgrp/chmod must precede setcap (they clear file capabilities)"
grep -q 'chmod 0750 "\$h"' "$inst" || fail "rssi-helper must be 0750"
echo "PASS security files"
