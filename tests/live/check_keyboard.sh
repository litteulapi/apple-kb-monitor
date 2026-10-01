#!/usr/bin/env bash
# Vérification de bout en bout, EN LECTURE SEULE, du clavier Bluetooth Apple connecté.
# Aucune écriture sur le clavier, aucun sudo, aucun service touché.
#
# Usage : tests/live/check_keyboard.sh [--mac AA:BB:..] [--tolerance N] [--strict] [--bin PATH] [--quiet]
# Sortie : une ligne PASS/FAIL/WARN/SKIP par contrôle. Code retour : 0 si aucun FAIL
# (avec --strict, WARN compte aussi comme échec) ; 1 sinon ; 2 erreur d'usage/clavier absent.
set -u

MAC="${KB_MAC:-04:DB:56:CA:42:EE}"
TOL="${KB_TOLERANCE:-5}"          # écart max toléré entre sources, en points de %
STRICT=0
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
BIN="${KB_BIN:-$ROOT/apple-kb-monitor}"
RUST_BIN="${KB_RUST_BIN:-$ROOT/apihub-app/target/release/apihub-app}"
UDEV_RULE="$ROOT/udev/70-apple-kb-hidraw.rules"
NPASS=0; NFAIL=0; NWARN=0; NSKIP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --mac) MAC="$2"; shift 2 ;;
    --tolerance) TOL="$2"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    --bin) BIN="$2"; shift 2 ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "option inconnue: $1" >&2; exit 2 ;;
  esac
done
case "$TOL" in ''|*[!0-9]*) echo "tolerance invalide: $TOL" >&2; exit 2 ;; esac

MAC_UP="$(printf '%s' "$MAC" | tr 'a-f' 'A-F')"
MAC_LO="$(printf '%s' "$MAC" | tr 'A-F' 'a-f')"
MAC_US="${MAC_UP//:/_}"                         # 04_DB_56_...
MAC_UPOWER="$(printf '%s' "$MAC_LO" | sed 's/:/o/g')"  # 04odbo56...

pass() { NPASS=$((NPASS+1)); printf 'PASS  %-28s %s\n' "$1" "${2:-}"; }
fail() { NFAIL=$((NFAIL+1)); printf 'FAIL  %-28s %s\n' "$1" "${2:-}"; }
warn() { NWARN=$((NWARN+1)); printf 'WARN  %-28s %s\n' "$1" "${2:-}"; }
skip() { NSKIP=$((NSKIP+1)); printf 'SKIP  %-28s %s\n' "$1" "${2:-}"; }
is_num() { case "${1:-}" in ''|*[!0-9]*) return 1 ;; *) return 0 ;; esac; }

# ---- 1. connexion BlueZ -------------------------------------------------
INFO="$(bluetoothctl info "$MAC_UP" 2>&1)"
if printf '%s' "$INFO" | grep -q 'Connected: yes'; then
  pass bluez_connected "$MAC_UP"
else
  fail bluez_connected "clavier $MAC_UP non connecté ou inconnu"
  echo "RESULT pass=$NPASS fail=$NFAIL warn=$NWARN skip=$NSKIP"; exit 2
fi
printf '%s' "$INFO" | grep -q 'Paired: yes' && printf '%s' "$INFO" | grep -q 'Trusted: yes' \
  && pass bluez_paired_trusted || warn bluez_paired_trusted "non apparié/trusted"
MODALIAS="$(printf '%s' "$INFO" | sed -n 's/^[[:space:]]*Modalias: //p')"
case "$MODALIAS" in
  usb:v05AC*|bluetooth:v004C*) pass apple_vendor "$MODALIAS" ;;
  *) fail apple_vendor "modalias inattendu: ${MODALIAS:-vide}" ;;
esac

# ---- 2. découverte hidraw / evdev / power_supply depuis le MAC ----------
HIDRAW=""; HIDDEV=""
for h in /sys/class/hidraw/hidraw*; do
  [ -e "$h" ] || continue
  uniq="$(sed -n 's/^HID_UNIQ=//p' "$h/device/uevent" 2>/dev/null | tr 'A-F' 'a-f')"
  if [ "$uniq" = "$MAC_LO" ]; then HIDRAW="/dev/$(basename "$h")"; HIDDEV="$(readlink -f "$h/device")"; break; fi
done
if [ -n "$HIDRAW" ]; then pass hidraw_found "$HIDRAW"; else fail hidraw_found "aucun hidraw avec HID_UNIQ=$MAC_LO"; fi

EVDEV=""
if [ -n "$HIDDEV" ]; then
  for e in "$HIDDEV"/input/input*/event*; do [ -e "$e" ] && { EVDEV="/dev/input/$(basename "$e")"; break; }; done
fi
if [ -n "$EVDEV" ] && [ -e "$EVDEV" ]; then pass evdev_found "$EVDEV"; else fail evdev_found "pas d'event pour $MAC_LO"; fi

PS=""
for p in /sys/class/power_supply/hid-"$MAC_LO"-battery*; do [ -d "$p" ] && { PS="$p"; break; }; done
if [ -n "$PS" ]; then pass power_supply_found "$PS"; else fail power_supply_found "aucun /sys/class/power_supply/hid-$MAC_LO-battery*"; fi

# ---- 3. permissions hidraw (accès utilisateur sans sudo) ----------------
if [ -n "$HIDRAW" ]; then
  if [ -r "$HIDRAW" ] && [ -w "$HIDRAW" ]; then pass hidraw_access "rw pour $(id -un)"
  else fail hidraw_access "$HIDRAW non accessible en rw pour $(id -un) ($(stat -c '%A %U:%G' "$HIDRAW")) — GET_REPORT impossible"; fi
fi
if [ -r "$UDEV_RULE" ]; then
  inst=""
  for d in /etc/udev/rules.d /usr/lib/udev/rules.d /lib/udev/rules.d; do
    [ -f "$d/70-apple-kb-hidraw.rules" ] && inst="$d/70-apple-kb-hidraw.rules"
  done
  if [ -z "$inst" ]; then fail udev_rule_installed "70-apple-kb-hidraw.rules absente de /etc|/usr/lib/udev/rules.d"
  elif cmp -s "$UDEV_RULE" "$inst"; then pass udev_rule_installed "$inst identique au dépôt"
  else warn udev_rule_installed "$inst diffère de $UDEV_RULE"; fi
else
  skip udev_rule_installed "règle du dépôt introuvable"
fi
if [ -n "$HIDRAW" ]; then
  tags="$(udevadm info -q property "$HIDRAW" 2>/dev/null | sed -n 's/^CURRENT_TAGS=//p')"
  case "$tags" in *uaccess*) pass udev_uaccess_tag "$tags" ;; *) fail udev_uaccess_tag "tag uaccess absent (CURRENT_TAGS=${tags:-vide})" ;; esac
  kern="$(udevadm info -a "$HIDRAW" 2>/dev/null | sed -n 's/^[[:space:]]*KERNELS=="\(0005:[0-9A-Fa-f]*:[^"]*\)"/\1/p' | head -1)"
  case "$kern" in 0005:05AC:*|0005:004C:*) pass udev_rule_matches_kernels "$kern" ;; *) fail udev_rule_matches_kernels "KERNELS=${kern:-vide} hors des motifs de la règle" ;; esac
fi

# ---- 4. pourcentage batterie : sysfs / UPower / BlueZ / script / Rust ---
SYS=""; UP=""; BZ=""; PYV=""
[ -n "$PS" ] && SYS="$(cat "$PS/capacity" 2>/dev/null)"
if is_num "$SYS"; then pass battery_sysfs "${SYS}% ($(cat "$PS/status" 2>/dev/null))"; else fail battery_sysfs "capacity illisible"; fi

if command -v upower >/dev/null 2>&1; then
  UPP="$(upower -e 2>/dev/null | grep -i "$MAC_UPOWER" | head -1)"
  if [ -n "$UPP" ]; then
    UP="$(upower -i "$UPP" 2>/dev/null | sed -n 's/^[[:space:]]*percentage:[[:space:]]*\([0-9]*\)%.*/\1/p' | head -1)"
    if is_num "$UP"; then pass battery_upower "${UP}%"; else fail battery_upower "pourcentage illisible ($UPP)"; fi
  else fail battery_upower "device UPower absent"; fi
else skip battery_upower "upower non installé"; fi

BZOUT="$(busctl --system get-property org.bluez "/org/bluez/hci0/dev_$MAC_US" org.bluez.Battery1 Percentage 2>&1)"
case "$BZOUT" in
  "y "*) BZ="${BZOUT#y }"; pass battery_bluez "${BZ}%" ;;
  *"No such interface"*)
    if systemctl --user is-active --quiet apple-kb-monitor 2>/dev/null || systemctl is-active --quiet apple-kb-monitor 2>/dev/null; then
      fail battery_bluez "daemon actif mais Battery1 non enregistré"
    else warn battery_bluez "Battery1 absent (daemon apple-kb-monitor inactif : fournisseur non enregistré)"; fi ;;
  *) warn battery_bluez "lecture D-Bus impossible: ${BZOUT:0:80}" ;;
esac

JSON=""
if [ -x "$BIN" ] && command -v python3 >/dev/null 2>&1; then
  JSON="$(timeout 30 "$BIN" --json 2>/dev/null)"
  PYOUT="$(printf '%s' "$JSON" | python3 -c '
import json,sys
try: d=json.load(sys.stdin)
except Exception as e: print("ERR json invalide: %s"%e); sys.exit()
b=d.get("battery",{}) or {}
print("PCT", b.get("percentage") if b.get("percentage") is not None else "null")
print("SYSFS", len(d.get("sysfs") or {}))
print("HIDRAW", (d.get("device") or {}).get("hidraw"))
print("EVDEV", (d.get("device") or {}).get("evdev"))
print("RSSI", (d.get("radio") or {}).get("rssi_dbm"))
' 2>&1)"
  case "$PYOUT" in
    ERR*) fail script_json "$PYOUT" ;;
    *)
      PYV="$(printf '%s\n' "$PYOUT" | sed -n 's/^PCT //p')"
      PYSYS="$(printf '%s\n' "$PYOUT" | sed -n 's/^SYSFS //p')"
      PYHID="$(printf '%s\n' "$PYOUT" | sed -n 's/^HIDRAW //p')"
      PYEV="$(printf '%s\n' "$PYOUT" | sed -n 's/^EVDEV //p')"
      pass script_json "JSON valide"
      [ "$PYHID" = "$HIDRAW" ] && pass script_hidraw_agrees "$PYHID" || fail script_hidraw_agrees "script=$PYHID attendu=$HIDRAW"
      [ "$PYEV" = "$EVDEV" ] && pass script_evdev_agrees "$PYEV" || fail script_evdev_agrees "script=$PYEV attendu=$EVDEV"
      if [ "${PYSYS:-0}" -gt 0 ] 2>/dev/null; then pass script_sysfs_populated "$PYSYS attributs"
      else fail script_sysfs_populated "bloc sysfs vide alors que $PS existe (chemin ps_path sans suffixe -NN ?)"; fi
      if is_num "$PYV"; then pass battery_script "${PYV}%"
      else fail battery_script "battery.percentage=null (hidraw illisible et pas de repli sysfs ?)"; PYV=""; fi ;;
  esac
else
  skip script_json "$BIN absent ou python3 manquant"
fi

if [ -x "$RUST_BIN" ]; then skip battery_rust "binaire présent mais sans mode CLI --json (GUI/tray) : non interrogé"
else skip battery_rust "binaire Rust non compilé ($RUST_BIN)"; fi

# comparaison des sources à la référence sysfs
cmp_src() { # nom valeur
  [ -z "$2" ] && return 0
  is_num "$SYS" || return 0
  d=$(( $2 > SYS ? $2 - SYS : SYS - $2 ))
  if [ "$d" -le "$TOL" ]; then pass "agree_$1_vs_sysfs" "écart ${d} <= ${TOL}"; else fail "agree_$1_vs_sysfs" "$1=$2% sysfs=${SYS}% écart ${d} > ${TOL}"; fi
}
cmp_src upower "$UP"; cmp_src bluez "$BZ"; cmp_src script "$PYV"

# ---- 5. keyd, services --------------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
  systemctl is-active --quiet keyd && pass keyd_active || fail keyd_active "keyd inactif"
  systemctl is-active --quiet bluetooth && pass bluetooth_active || fail bluetooth_active "bluetooth inactif"
  systemctl is-active --quiet upower && pass upower_active || warn upower_active "upower inactif"
  if systemctl --user is-active --quiet apple-kb-monitor 2>/dev/null || systemctl is-active --quiet apple-kb-monitor 2>/dev/null; then
    pass daemon_apple_kb_monitor active
  else warn daemon_apple_kb_monitor "service apple-kb-monitor inactif"; fi
fi
if [ -r /etc/keyd/apple-keyboard.conf ] || ls /etc/keyd/*.conf >/dev/null 2>&1; then
  if grep -qi '05ac:0256\|05ac:\*' /etc/keyd/*.conf 2>/dev/null; then pass keyd_config_ids "id Apple présent"
  else warn keyd_config_ids "aucun id 05ac dans /etc/keyd/*.conf"; fi
  if [ -f "$ROOT/keyd/apple-keyboard.conf" ] && [ -f /etc/keyd/apple-keyboard.conf ]; then
    cmp -s "$ROOT/keyd/apple-keyboard.conf" /etc/keyd/apple-keyboard.conf && pass keyd_config_synced || warn keyd_config_synced "/etc/keyd/apple-keyboard.conf diffère du dépôt"
  fi
else skip keyd_config_ids "pas de config keyd"; fi

echo "RESULT pass=$NPASS fail=$NFAIL warn=$NWARN skip=$NSKIP tolerance=${TOL}"
[ "$NFAIL" -gt 0 ] && exit 1
[ "$STRICT" -eq 1 ] && [ "$NWARN" -gt 0 ] && exit 1
exit 0
