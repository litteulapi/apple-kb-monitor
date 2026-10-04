#!/bin/sh
# shellcheck disable=SC2024  # only the reads need root; the snapshot files belong to the user
# PASSIVE snapshot of the keyboard's Bluetooth link (no write to the keyboard or to BlueZ).
# Usage : link_snapshot.sh [MAC] [OUT_DIR]
# Needs `sudo -n` for: BlueZ SDP cache, info file (key MASKED), debugfs, btmgmt (read).
# Does NOT start btmon: reuse an existing capture with link_btmon_stats.py.
set -eu
MAC=${1:-AA:BB:CC:DD:EE:F1}
OUT=${2:-.}
ADAPTER=$(cat /sys/class/bluetooth/hci0/address 2>/dev/null || btmgmt --index 0 info 2>/dev/null | awk '/addr/{print $2; exit}')
DEV=$(echo "$MAC" | tr ':' '_')
mkdir -p "$OUT"

bluetoothctl info "$MAC"                                   > "$OUT/link_bluetoothctl_info.txt" 2>&1 || true
busctl get-property org.bluez "/org/bluez/hci0/dev_$DEV" org.bluez.Input1 ReconnectMode \
                                                           > "$OUT/link_input1_reconnectmode.txt" 2>&1 || true
# SDP cache (no key in it).
sudo -n cat "/var/lib/bluetooth/$ADAPTER/cache/$MAC"       > "$OUT/link_sdp_cache.txt" 2>&1 || true
# Info file: link key masked BEFORE writing to disk.
sudo -n sed -E 's/^(Key|IRK|LTK|EncSize|Rand|EDiv)=.*/\1=<masked>/' \
     "/var/lib/bluetooth/$ADAPTER/$MAC/info"              > "$OUT/link_bluez_info_masked.txt" 2>&1 || true
# Host parameters (MGMT read: Get Connection Information = local Read RSSI + Read TX Power).
sudo -n btmgmt --index 0 conn-info -t 0 "$MAC"             > "$OUT/link_conn_info.txt" 2>&1 || true
sudo -n btmgmt --index 0 read-sysconfig                    > "$OUT/link_mgmt_sysconfig.txt" 2>&1 || true
sudo -n btmgmt --index 0 info                              > "$OUT/link_mgmt_info.txt" 2>&1 || true
# debugfs hci0 (read only; link_keys / long_term_keys / identity_resolving_keys are NOT read).
D=/sys/kernel/debug/bluetooth/hci0
for f in hci_version hci_revision manufacturer features idle_timeout sniff_min_interval \
         sniff_max_interval supervision_timeout conn_info_min_age conn_info_max_age \
         min_encrypt_key_size max_key_size; do
  printf '%s: ' "$f"; sudo -n cat "$D/$f" 2>&1 | tr '\n' ' '; echo
done                                                       > "$OUT/link_debugfs_hci0.txt"
sudo -n cat "$D/device_list" 2>&1 | grep -i "$MAC"         > "$OUT/link_accept_list.txt" || true
# Offline SDP decoding.
python3 "$(dirname "$0")/link_sdp_decode.py" "$OUT/link_sdp_cache.txt" > "$OUT/link_sdp_decoded.txt" 2>&1 || true
echo "snapshot written to $OUT"
