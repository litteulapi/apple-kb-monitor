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

# #248: the read-only MTU probe (`akm-hid-inspect`) asks no password in the
# active local session; the HID_CONTROL bytes keep asking for an administrator.
# One action each, told apart by their exec.path ALONE: pkexec takes the first
# action whose path (and argv1, if any) matches, in an order polkitd does not
# guarantee, so no two actions may share a path.
policy="$top/polkit/com.agenceapi.AppleKbMonitor.policy"
action() { awk -v id="$1" '$0 ~ "<action id=\"" id "\">" {p=1} p {print} p && /<\/action>/ {exit}' "$policy"; }
insp=$(action com.agenceapi.AppleKbMonitor.hid-inspect)
ctl=$(action com.agenceapi.AppleKbMonitor.hid-control)
[ -n "$insp" ] && [ -n "$ctl" ] || fail "polkit actions hid-inspect / hid-control missing"
path_of() { echo "$1" | sed -n 's|.*exec\.path">\([^<]*\)</annotate>.*|\1|p'; }
[ "$(path_of "$insp")" = /usr/lib/apple-kb-monitor/akm-hid-inspect ] \
  || fail "hid-inspect must be bound to its own executable akm-hid-inspect"
[ "$(path_of "$ctl")" = /usr/lib/apple-kb-monitor/akm-hid-control ] \
  || fail "hid-control must be bound to akm-hid-control"
if echo "$insp" | grep -q 'exec.argv1'; then fail "hid-inspect must be told apart by its path, not by an argv1"; fi
# Deterministic association: every exec.path of the policy belongs to ONE action.
dup=$(sed -n 's|.*exec\.path">\([^<]*\)</annotate>.*|\1|p' "$policy" | sort | uniq -d)
[ -z "$dup" ] || fail "several polkit actions share an exec.path (pkexec's choice is not deterministic): $dup"
if grep -q 'exec.argv1' "$policy"; then fail "no polkit action may depend on exec.argv1"; fi
# The executable of the password-less action exists, is packaged, and is the
# one akmctl runs; it has no verb and no send path (unit tests of the binary).
[ -f "$top/apihub-app/crates/akm-helper/src/bin/akm-hid-inspect.rs" ] || fail "akm-hid-inspect source missing"
grep -q 'target/release/akm-hid-inspect".*"\$pkgdir/usr/lib/apple-kb-monitor/akm-hid-inspect"' "$top/PKGBUILD" \
  || fail "PKGBUILD does not install akm-hid-inspect"
grep -qx 'usr/lib/apple-kb-monitor/akm-hid-inspect' "$top/scripts/package-expected.txt" \
  || fail "akm-hid-inspect missing from scripts/package-expected.txt"
grep -q 'HID_INSPECT_HELPER: &str = "/usr/lib/apple-kb-monitor/akm-hid-inspect"' "$top/apihub-app/crates/akmctl/src/hid_control.rs" \
  || fail "akmctl must run akm-hid-inspect for the MTU probe"
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
