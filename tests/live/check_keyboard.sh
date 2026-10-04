#!/usr/bin/env bash
# End-to-end check, READ ONLY, of the connected Apple Bluetooth keyboard.
# No write to the keyboard, no sudo, no service touched.
#
# Usage: tests/live/check_keyboard.sh [--mac AA:BB:..] [--tolerance N] [--strict] [--akmctl PATH] [--quiet]
# Without --mac: the first Apple Bluetooth keyboard seen in /sys/class/hidraw (KB_MAC as a variable).
# Output: one PASS/FAIL/WARN/SKIP line per check (--quiet: only FAIL/WARN + RESULT). Exit code: 0 if no FAIL
# (with --strict, WARN also counts as a failure); 1 otherwise; 2 usage error/keyboard absent.
set -u

MAC="${KB_MAC:-}"
QUIET=0
TOL="${KB_TOLERANCE:-5}"          # max tolerated gap between sources, in % points
STRICT=0
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
AKMCTL="${KB_AKMCTL:-}"
UDEV_RULE="$ROOT/udev/70-apple-kb-hidraw.rules"
NPASS=0; NFAIL=0; NWARN=0; NSKIP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --mac) MAC="$2"; shift 2 ;;
    --tolerance) TOL="$2"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    --akmctl) AKMCTL="$2"; shift 2 ;;
    --quiet) QUIET=1; shift ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
case "$TOL" in ''|*[!0-9]*) echo "invalid tolerance: $TOL" >&2; exit 2 ;; esac

# Auto-detected MAC: first Bluetooth hidraw from an Apple vendor (05AC / 004C).
if [ -z "$MAC" ]; then
  for h in /sys/class/hidraw/hidraw*; do
    [ -e "$h" ] || continue
    case "$(sed -n 's/^HID_ID=//p' "$h/device/uevent" 2>/dev/null)" in
      0005:0000004C:*|0005:000005AC:*) MAC="$(sed -n 's/^HID_UNIQ=//p' "$h/device/uevent" 2>/dev/null)"; break ;;
    esac
  done
  [ -n "$MAC" ] || { echo "no Apple Bluetooth keyboard detected (use --mac)" >&2; echo "RESULT pass=0 fail=1 warn=0 skip=0"; exit 2; }
fi
MAC_UP="$(printf '%s' "$MAC" | tr 'a-f' 'A-F')"
MAC_LO="$(printf '%s' "$MAC" | tr 'A-F' 'a-f')"
MAC_US="${MAC_UP//:/_}"                         # 04_DB_56_...
MAC_UPOWER="$(printf '%s' "$MAC_LO" | sed 's/:/o/g')"  # 04odbo56...

pass() { NPASS=$((NPASS+1)); [ "$QUIET" -eq 1 ] || printf 'PASS  %-28s %s\n' "$1" "${2:-}"; }
fail() { NFAIL=$((NFAIL+1)); printf 'FAIL  %-28s %s\n' "$1" "${2:-}"; }
warn() { NWARN=$((NWARN+1)); printf 'WARN  %-28s %s\n' "$1" "${2:-}"; }
skip() { NSKIP=$((NSKIP+1)); [ "$QUIET" -eq 1 ] || printf 'SKIP  %-28s %s\n' "$1" "${2:-}"; }
# Rust daemon (sole owner of the keyboard).
daemon_state() { # -> rust | none
  if systemctl --user is-active --quiet apple-kb-monitord 2>/dev/null; then echo rust
  else echo none; fi
}
DAEMON="$(daemon_state)"
is_num() { case "${1:-}" in ''|*[!0-9]*) return 1 ;; *) return 0 ;; esac; }

# ---- 1. BlueZ connection------------------------------------------------
INFO="$(bluetoothctl info "$MAC_UP" 2>&1)"
if printf '%s' "$INFO" | grep -q 'Connected: yes'; then
  pass bluez_connected "$MAC_UP"
else
  fail bluez_connected "keyboard $MAC_UP not connected or unknown"
  echo "RESULT pass=$NPASS fail=$NFAIL warn=$NWARN skip=$NSKIP"; exit 2
fi
if printf '%s' "$INFO" | grep -q 'Paired: yes' && printf '%s' "$INFO" | grep -q 'Trusted: yes'; then
  pass bluez_paired_trusted
else
  warn bluez_paired_trusted "not paired/trusted"
fi
MODALIAS="$(printf '%s' "$INFO" | sed -n 's/^[[:space:]]*Modalias: //p')"
case "$MODALIAS" in
  usb:v05AC*|bluetooth:v004C*) pass apple_vendor "$MODALIAS" ;;
  *) fail apple_vendor "unexpected modalias: ${MODALIAS:-empty}" ;;
esac

# ---- 2. hidraw / evdev / power_supply discovery from the MAC ----------
HIDRAW=""; HIDDEV=""
for h in /sys/class/hidraw/hidraw*; do
  [ -e "$h" ] || continue
  uniq="$(sed -n 's/^HID_UNIQ=//p' "$h/device/uevent" 2>/dev/null | tr 'A-F' 'a-f')"
  if [ "$uniq" = "$MAC_LO" ]; then HIDRAW="/dev/$(basename "$h")"; HIDDEV="$(readlink -f "$h/device")"; break; fi
done
if [ -n "$HIDRAW" ]; then pass hidraw_found "$HIDRAW"; else fail hidraw_found "no hidraw with HID_UNIQ=$MAC_LO"; fi

EVDEV=""
if [ -n "$HIDDEV" ]; then
  for e in "$HIDDEV"/input/input*/event*; do [ -e "$e" ] && { EVDEV="/dev/input/$(basename "$e")"; break; }; done
fi
if [ -n "$EVDEV" ] && [ -e "$EVDEV" ]; then pass evdev_found "$EVDEV"; else fail evdev_found "no event for $MAC_LO"; fi

PS=""
for p in /sys/class/power_supply/hid-"$MAC_LO"-battery*; do [ -d "$p" ] && { PS="$p"; break; }; done
if [ -n "$PS" ]; then pass power_supply_found "$PS"; else fail power_supply_found "no /sys/class/power_supply/hid-$MAC_LO-battery*"; fi

# ---- 3. hidraw permissions (user access without sudo) ----------------
if [ -n "$HIDRAW" ]; then
  if [ -r "$HIDRAW" ] && [ -w "$HIDRAW" ]; then pass hidraw_access "rw for $(id -un)"
  else fail hidraw_access "$HIDRAW not accessible in rw for $(id -un) ($(stat -c '%A %U:%G' "$HIDRAW")) — GET_REPORT impossible"; fi
fi
if [ -r "$UDEV_RULE" ]; then
  inst=""
  for d in /etc/udev/rules.d /usr/lib/udev/rules.d /lib/udev/rules.d; do
    [ -f "$d/70-apple-kb-hidraw.rules" ] && inst="$d/70-apple-kb-hidraw.rules"
  done
  if [ -z "$inst" ]; then fail udev_rule_installed "70-apple-kb-hidraw.rules missing from /etc|/usr/lib/udev/rules.d"
  elif cmp -s "$UDEV_RULE" "$inst"; then pass udev_rule_installed "$inst identical to the repository"
  else warn udev_rule_installed "$inst differs from $UDEV_RULE"; fi
else
  skip udev_rule_installed "repository rule not found"
fi
if [ -n "$HIDRAW" ]; then
  tags="$(udevadm info -q property "$HIDRAW" 2>/dev/null | sed -n 's/^CURRENT_TAGS=//p')"
  case "$tags" in *uaccess*) pass udev_uaccess_tag "$tags" ;; *) fail udev_uaccess_tag "uaccess tag missing (CURRENT_TAGS=${tags:-empty})" ;; esac
  kern="$(udevadm info -a "$HIDRAW" 2>/dev/null | sed -n 's/^[[:space:]]*KERNELS=="\(0005:[0-9A-Fa-f]*:[^"]*\)"/\1/p' | head -1)"
  case "$kern" in 0005:05AC:*|0005:004C:*) pass udev_rule_matches_kernels "$kern" ;; *) fail udev_rule_matches_kernels "KERNELS=${kern:-empty} outside the rule's patterns" ;; esac
fi

# ---- 4. battery percentage: sysfs / UPower / BlueZ / script / Rust ---
SYS=""; UP=""; BZ=""
[ -n "$PS" ] && SYS="$(cat "$PS/capacity" 2>/dev/null)"
if is_num "$SYS"; then pass battery_sysfs "${SYS}% ($(cat "$PS/status" 2>/dev/null))"; else fail battery_sysfs "capacity unreadable"; fi

if command -v upower >/dev/null 2>&1; then
  UPP="$(upower -e 2>/dev/null | grep -i "$MAC_UPOWER" | head -1)"
  if [ -n "$UPP" ]; then
    UP="$(upower -i "$UPP" 2>/dev/null | sed -n 's/^[[:space:]]*percentage:[[:space:]]*\([0-9]*\)%.*/\1/p' | head -1)"
    if is_num "$UP"; then pass battery_upower "${UP}%"; else fail battery_upower "percentage unreadable ($UPP)"; fi
  else fail battery_upower "UPower device missing"; fi
else skip battery_upower "upower not installed"; fi

BZOUT="$(busctl --system get-property org.bluez "/org/bluez/hci0/dev_$MAC_US" org.bluez.Battery1 Percentage 2>&1)"
case "$BZOUT" in
  "y "*) BZ="${BZOUT#y }"; pass battery_bluez "${BZ}%" ;;
  *"No such interface"*)
    if [ "$DAEMON" != none ]; then
      fail battery_bluez "daemon ($DAEMON) active but Battery1 not registered"
    else warn battery_bluez "Battery1 absent (apple-kb-monitord inactive: provider not registered)"; fi ;;
  *) warn battery_bluez "D-Bus read failed: ${BZOUT:0:80}" ;;
esac

# Rust daemon: akmctl status --json (D-Bus session read, no write).
RS=""
if [ -z "$AKMCTL" ]; then
  if command -v akmctl >/dev/null 2>&1; then AKMCTL="$(command -v akmctl)"
  elif [ -x "$ROOT/apihub-app/target/release/akmctl" ]; then AKMCTL="$ROOT/apihub-app/target/release/akmctl"; fi
fi
if [ "$DAEMON" != rust ]; then
  skip battery_rust "apple-kb-monitord inactive"
elif [ -z "$AKMCTL" ] || [ ! -x "$AKMCTL" ]; then
  skip battery_rust "akmctl not found"
else
  if busctl --user status com.agenceapi.AppleKbMonitor1 >/dev/null 2>&1; then pass daemon_dbus_name "com.agenceapi.AppleKbMonitor1 on the session bus"
  else fail daemon_dbus_name "apple-kb-monitord active but D-Bus name missing"; fi
  RSOUT="$(timeout 15 "$AKMCTL" status --json 2>/dev/null | python3 -c '
import json,sys
try: d=json.load(sys.stdin)
except Exception as e: print("ERR invalid json: %s"%e); sys.exit()
print("DAEMON", d.get("daemon")); print("CONN", d.get("connected")); print("PCT", d.get("battery_pct") if d.get("battery_pct") is not None else "null"); print("MAC", d.get("mac"))
' 2>&1)"
  case "$RSOUT" in
    ERR*|"") fail rust_status_json "${RSOUT:-empty output}" ;;
    *)
      RS="$(printf '%s\n' "$RSOUT" | sed -n 's/^PCT //p')"
      RMAC="$(printf '%s\n' "$RSOUT" | sed -n 's/^MAC //p' | tr 'a-f' 'A-F')"
      pass rust_status_json "akmctl status --json valid"
      if [ "$RMAC" = "$MAC_UP" ]; then pass rust_mac_agrees "$RMAC"; else warn rust_mac_agrees "daemon=$RMAC expected=$MAC_UP"; fi
      if is_num "$RS"; then pass battery_rust "${RS}%"; else fail battery_rust "battery_pct=null"; RS=""; fi ;;
  esac
fi

# comparison of the sources with the sysfs reference
cmp_src() { # name value
  [ -z "$2" ] && return 0
  is_num "$SYS" || return 0
  d=$(( $2 > SYS ? $2 - SYS : SYS - $2 ))
  if [ "$d" -le "$TOL" ]; then pass "agree_$1_vs_sysfs" "gap ${d} <= ${TOL}"; else fail "agree_$1_vs_sysfs" "$1=$2% sysfs=${SYS}% gap ${d} > ${TOL}"; fi
}
cmp_src upower "$UP"; cmp_src bluez "$BZ"; cmp_src rust "$RS"

# ---- 5. keyd, services --------------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
  # keyd is optional: warn only when an Apple keyd config is installed.
  if systemctl is-active --quiet keyd; then pass keyd_active
  elif grep -qi '05ac:0256\|05ac:\*' /etc/keyd/*.conf 2>/dev/null; then warn keyd_active "keyd inactive but an Apple keyd config is installed"
  else skip keyd_active "keyd not running (optional)"; fi
  if systemctl is-active --quiet bluetooth; then pass bluetooth_active; else fail bluetooth_active "bluetooth inactive"; fi
  if systemctl is-active --quiet upower; then pass upower_active; else warn upower_active "upower inactive"; fi
  case "$DAEMON" in
    rust) pass daemon_apple_kb_monitord "apple-kb-monitord active (session)" ;;
    *) warn daemon_apple_kb_monitord "apple-kb-monitord inactive (systemctl --user enable --now apple-kb-monitord)" ;;
  esac
fi
if [ -r /etc/keyd/apple-keyboard.conf ] || ls /etc/keyd/*.conf >/dev/null 2>&1; then
  if grep -qi '05ac:0256\|05ac:\*' /etc/keyd/*.conf 2>/dev/null; then pass keyd_config_ids "Apple id present"
  else warn keyd_config_ids "no 05ac id in /etc/keyd/*.conf"; fi
  if [ -f "$ROOT/keyd/apple-keyboard.conf" ] && [ -f /etc/keyd/apple-keyboard.conf ]; then
    if cmp -s "$ROOT/keyd/apple-keyboard.conf" /etc/keyd/apple-keyboard.conf; then pass keyd_config_synced; else warn keyd_config_synced "/etc/keyd/apple-keyboard.conf differs from the repository"; fi
  fi
else skip keyd_config_ids "no keyd config"; fi

echo "RESULT pass=$NPASS fail=$NFAIL warn=$NWARN skip=$NSKIP tolerance=${TOL}"
[ "$NFAIL" -gt 0 ] && exit 1
[ "$STRICT" -eq 1 ] && [ "$NWARN" -gt 0 ] && exit 1
exit 0
