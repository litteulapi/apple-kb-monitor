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

# #248: the read-only MTU probe (`akm-hid-control inspect`) asks no password in
# the active local session; the HID_CONTROL bytes keep asking for an
# administrator. One action each, told apart by `exec.argv1 = inspect`.
policy="$top/polkit/com.agenceapi.AppleKbMonitor.policy"
action() { awk -v id="$1" '$0 ~ "<action id=\"" id "\">" {p=1} p {print} p && /<\/action>/ {exit}' "$policy"; }
insp=$(action com.agenceapi.AppleKbMonitor.hid-inspect)
ctl=$(action com.agenceapi.AppleKbMonitor.hid-control)
[ -n "$insp" ] && [ -n "$ctl" ] || fail "polkit actions hid-inspect / hid-control missing"
echo "$insp" | grep -q '<annotate key="org.freedesktop.policykit.exec.argv1">inspect</annotate>' \
  || fail "hid-inspect must be bound to argv1 = inspect"
echo "$insp" | grep -q '<allow_active>yes</allow_active>' || fail "hid-inspect must be allow_active = yes"
echo "$insp" | grep -q '<allow_any>no</allow_any>' || fail "hid-inspect must stay refused outside the active local session"
echo "$insp" | grep -q '<allow_inactive>no</allow_inactive>' || fail "hid-inspect must stay refused for inactive sessions"
echo "$ctl" | grep -q '<allow_active>auth_admin</allow_active>' || fail "hid-control must stay auth_admin"
if echo "$ctl" | grep -q 'exec.argv1'; then fail "hid-control must not be bound to an argv1"; fi
if echo "$ctl" | grep -q '>yes<'; then fail "hid-control must never be allowed without authentication"; fi

# #209: rssi-helper restricted to group akm, capability applied last.
grep -q "sysusers.d/apple-kb-monitor.conf" "$top/PKGBUILD" || fail "PKGBUILD does not ship the akm group"
grep -qx "g akm - -" "$top/sysusers/apple-kb-monitor.conf" || fail "sysusers entry"
inst="$top/apple-kb-monitor.install"
c=$(grep -n 'chgrp akm' "$inst" | head -1 | cut -d: -f1)
s=$(grep -n 'setcap cap_net_admin+ep "\$h"' "$inst" | head -1 | cut -d: -f1)
[ -n "$c" ] && [ -n "$s" ] && [ "$c" -lt "$s" ] || fail "chgrp/chmod must precede setcap (they clear file capabilities)"
grep -q 'chmod 0750 "\$h"' "$inst" || fail "rssi-helper must be 0750"
echo "PASS security files"
