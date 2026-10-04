#!/bin/sh
# Static guards of the security fixes. No hardware,
# no root, nothing installed. Run from anywhere: sh tests/check-security-files.sh
set -eu
top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }
unit="$top/systemd/apple-kb-monitord.service"

# the daemon starts pkexec (setuid) and rssi-helper (file capability).
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

# no retained authorization (the subject is the long-lived daemon).
if grep -E "<allow_[a-z]+>auth_admin_keep<" "$top/polkit/com.agenceapi.AppleKbMonitor.policy" >/dev/null; then
  fail "polkit action must not use auth_admin_keep"
fi

# One privileged program, akm-helper: every action is bound to its path AND
# to an argv1 equal to the command, so pkexec picks exactly one action per
# command (polkit 127); an unknown argv1 falls back to org.freedesktop.policykit.exec.
policy="$top/polkit/com.agenceapi.AppleKbMonitor.policy"
action() { awk -v id="$1" '$0 ~ "<action id=\"" id "\">" {p=1} p {print} p && /<\/action>/ {exit}' "$policy"; }
path_of() { echo "$1" | sed -n 's|.*exec\.path">\([^<]*\)</annotate>.*|\1|p'; }
argv1_of() { echo "$1" | sed -n 's|.*exec\.argv1">\([^<]*\)</annotate>.*|\1|p'; }
cmds="set-fnmode install-keymap hid-control hid-inspect doctor-fix"
[ "$(grep -c '<action id=' "$policy")" = 5 ] || fail "5 polkit actions expected"
for c in $cmds; do
  a=$(action "com.agenceapi.AppleKbMonitor.$c")
  [ -n "$a" ] || fail "polkit action $c missing"
  [ "$(path_of "$a")" = /usr/lib/apple-kb-monitor/akm-helper ] || fail "$c must be bound to akm-helper"
  [ "$(argv1_of "$a")" = "$c" ] || fail "$c must carry exec.argv1 = $c"
  echo "$a" | grep -q '<allow_any>no</allow_any>' || fail "$c: refused outside the active local session"
  echo "$a" | grep -q '<allow_inactive>no</allow_inactive>' || fail "$c: refused for inactive sessions"
  echo "$a" | grep -q '<description xml:lang="fr">' || fail "$c: no French description (R28)"
  echo "$a" | grep -q '<message xml:lang="fr">' || fail "$c: no French message (R28)"
  grep -q "\"$c\" => " "$top/apihub-app/crates/akm-helper/src/main.rs" || fail "akm-helper does not dispatch $c"
  if [ "$c" = hid-inspect ]; then
    echo "$a" | grep -q '<allow_active>yes</allow_active>' || fail "hid-inspect is read-only: allow_active yes"
  else
    echo "$a" | grep -q '<allow_active>auth_admin</allow_active>' || fail "$c must be auth_admin"
  fi
done
[ "$(sed -n 's|.*exec\.argv1">\([^<]*\)</annotate>.*|\1|p' "$policy" | sort | uniq -d)" = "" ] || fail "two actions share an argv1"
# a single helper binary, packaged, and the one the callers run
[ ! -d "$top/apihub-app/crates/akm-helper/src/bin" ] || fail "akm-helper must build one executable (src/bin/ found)"
# shellcheck disable=SC2016  # literal PKGBUILD text
grep -q 'target/release/akm-helper".*"\$pkgdir/usr/lib/apple-kb-monitor/akm-helper"' "$top/PKGBUILD" || fail "PKGBUILD does not install akm-helper"
if grep -qE 'akm-(hid-inspect|hid-control|keymap-helper|doctor-fix)' "$top/PKGBUILD" "$top/scripts/package-expected.txt" "$top/systemd/"*.service; then
  fail "an old helper executable is still referenced"
fi
# One definition of the helper path, shared by the daemon, akmctl and akm-helper.
grep -q 'pub const HELPER: &str = "/usr/lib/apple-kb-monitor/akm-helper";' "$top/apihub-app/akm-core/src/paths.rs" || fail "helper path"
grep -q 'use akm_core::paths::{HELPER, PKEXEC};' "$top/apihub-app/crates/akmctl/src/hid_control.rs" || fail "akmctl hid-inspect path"
grep -q 'use akm_core::paths::{HELPER, PKEXEC};' "$top/apihub-app/crates/akmctl/src/doctorfix.rs" || fail "doctor-fix path"
grep -q 'path = "../../akm-helper/src/doctor_fix.rs"' "$top/apihub-app/crates/akmctl/src/main.rs" \
  || fail "akmctl must take the list of corrections from akm-helper/src/doctor_fix.rs"
# no shell, no environment, no free path in the doctor-fix code of the helper
hsrc="$top/apihub-app/crates/akm-helper/src"
for f in "$hsrc/cmd/doctor_fix.rs" "$hsrc/doctor_fix.rs" "$hsrc/doctor_apply.rs"; do
  code=$(sed -e 's|^[[:space:]]*//.*$||' "$f" | sed -n '/#\[cfg(test)\]/q;p')
  if echo "$code" | grep -nE '"(/usr)?/bin/(ba|z|da)?sh"|"sh"|"-c"|env::var|Command::new|stdin\(\)' ; then
    fail "$(basename "$f"): shell, environment, stdin or direct process start in the doctor-fix helper"
  fi
done
grep -q 'vec!\[SYSTEMCTL, "restart", unit\]' "$hsrc/doctor_apply.rs" || fail "doctor_apply: systemctl restart must keep its fixed argument list"

# No group; rssi-helper root:root 0755 with its capability set in package().
[ ! -e "$top/sysusers/apple-kb-monitor.conf" ] || fail "sysusers file still present"
if grep -nE 'sysusers|chgrp|chmod 0750' "$top/PKGBUILD" "$top/apple-kb-monitor.install" | grep -v ':[[:space:]]*#'; then
  fail "no group handling may remain"
fi
# shellcheck disable=SC2016  # literal PKGBUILD text
grep -q 'install -Dm755 "$startdir/rssi-helper"' "$top/PKGBUILD" || fail "rssi-helper must be 0755"
# shellcheck disable=SC2016  # literal PKGBUILD text
grep -q 'setcap cap_net_admin+ep "$pkgdir/usr/lib/apple-kb-monitor/rssi-helper"' "$top/PKGBUILD" || fail "setcap in package()"
sh "$top/tests/check-rssi-helper.sh" >/dev/null || fail "rssi-helper refusal tests"
# user units cannot order on system targets (silently ignored by the user manager)
for u in "$top"/systemd/apple-kb-monitord.service "$top"/systemd/apple-kb-monitor-shutdown.service "$top"/systemd/apple-kb-monitor-selfcheck.*; do
  grep -qE '^(After|Before|Requires|Wants)=.*bluetooth\.target' "$u" && fail "$(basename "$u"): bluetooth.target is a system unit"
done
echo "PASS security files"
