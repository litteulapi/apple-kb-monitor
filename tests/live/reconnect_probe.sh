#!/usr/bin/env bash
# Controlled reconnection test of the Apple keyboard (docs/RECONNECTION-PAIRING.md §7).
#
# ONE `bluetoothctl disconnect`, then the person at the keyboard presses ONE key
# and the script records, read-only, what the radio, the adapter's USB power
# state and bluetoothd do. Nothing else is written: no remove, no unpair, no
# SET_REPORT, no service restart. btmon needs sudo (read-only capture).
#
#   tests/live/reconnect_probe.sh [MAC] [--out DIR] [--idle SECONDS] [--wait SECONDS]
#
# Requirements: the keyboard is connected now, someone is at the keyboard, and
# another input device is at hand in case it does not come back.
# Recovery if it does not come back: press a key again; `akmctl repair`.
set -euo pipefail

MAC=AA:BB:CC:DD:EE:F1
OUT="${XDG_STATE_HOME:-$HOME/.local/state}/apple-kb-monitor/probe-$(date +%Y%m%d-%H%M%S)"
IDLE=60     # seconds of silence before the key press (lets the adapter autosuspend)
WAIT=180    # max seconds to wait for the reconnection after the prompt
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT=$2; shift 2 ;;
        --idle) IDLE=$2; shift 2 ;;
        --wait) WAIT=$2; shift 2 ;;
        -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
        *) MAC=$1; shift ;;
    esac
done
mkdir -p "$OUT"
log() { printf '%s %s\n' "$(date +%T)" "$*" | tee -a "$OUT/probe.log"; }

connected() { bluetoothctl info "$MAC" 2>/dev/null | grep -q 'Connected: yes'; }
usbdir() {
    local d
    d=$(readlink -f /sys/class/bluetooth/hci0/device 2>/dev/null) || return 1
    [ -f "$d/idVendor" ] || d=$(dirname "$d")
    echo "$d"
}

connected || { echo "the keyboard $MAC is not connected: nothing to test" >&2; exit 2; }
sudo -n true 2>/dev/null || { echo "sudo -n needed for btmon (read-only capture)" >&2; exit 2; }
U=$(usbdir) || U=""

echo "Test: ONE disconnection of $MAC, then ONE key press after ${IDLE} s."
read -r -p "Is someone at the keyboard, with another input device at hand? [type yes] " a
[ "$a" = yes ] || { echo "cancelled"; exit 1; }

sudo -n btmon -w "$OUT/btmon.snoop" >/dev/null 2>&1 &
BTMON=$!
trap 'sudo -n kill $BTMON 2>/dev/null || true' EXIT
sleep 1
T0=$(date --iso-8601=seconds)
log "start; usb=${U:-none} control=$( [ -n "$U" ] && cat "$U/power/control")"

log "bluetoothctl disconnect $MAC"
bluetoothctl disconnect "$MAC" >>"$OUT/probe.log" 2>&1 || true

sample() { [ -n "$U" ] && printf '%s %s susp_ms=%s\n' "$(date +%T.%N | cut -c1-12)" \
    "$(cat "$U/power/runtime_status")" "$(cat "$U/power/runtime_suspended_time")" >>"$OUT/usb.log"; true; }

for _ in $(seq 1 "$IDLE"); do sample; sleep 1; done
log ">>> PRESS ONE KEY NOW <<<"
printf '\a'
T_KEY=$(date +%s)
BACK=""
for _ in $(seq 1 $((WAIT * 4))); do
    sample
    if connected; then BACK=$(( $(date +%s) - T_KEY )); break; fi
    sleep 0.25
done
if [ -n "$BACK" ]; then log "reconnected ${BACK} s after the prompt"; else log "NOT reconnected after ${WAIT} s"; fi
sleep 3
sudo -n kill $BTMON 2>/dev/null || true
wait $BTMON 2>/dev/null || true
trap - EXIT

journalctl -u bluetooth --since "$T0" --no-pager -o short-iso >"$OUT/bluetoothd.log" 2>/dev/null || true
# shellcheck disable=SC2024  # only the read needs root; the text file belongs to the user
btmon -r "$OUT/btmon.snoop" -T >"$OUT/btmon.txt" 2>/dev/null || sudo -n btmon -r "$OUT/btmon.snoop" -T >"$OUT/btmon.txt"

T="$OUT/btmon.txt"
n() { grep -c -E "$1" "$T" || true; }
{
    echo "== summary ($MAC) =="
    echo "incoming Connection Request (keyboard paged us): $(n 'Conn(ect|ection) Request \(0x04\)')"
    echo "host Create Connection (we paged it)           : $(n 'Create Connection \(0x01[|]0x0005\)')"
    echo "Connection Complete success / failure          : $(grep -E -A3 'Connect(ion)? Complete \(0x03\)' "$T" | grep -c 'Status: Success' || true) / $(grep -E -A3 'Connect(ion)? Complete \(0x03\)' "$T" | grep 'Status:' | grep -vc Success || true)"
    echo "Page Timeout                                   : $(n 'Page Timeout')"
    echo "Link Key Request                               : $(n 'Link Key Request \(0x17\)')"
    echo "Link Key Request Reply / Negative Reply        : $(n 'Link Key Request Reply \(0x01[|]0x000b\)') / $(n 'Link Key Request Neg')"
    echo "Authentication failure / PIN or Key Missing    : $(n 'Authentication Failure') / $(n 'PIN or Key Missing')"
    echo "Accept Connection Request (host accepted)      : $(n 'Accept Connection Request')"
    echo "Reject Connection Request                      : $(n 'Reject Connection Request')"
    echo "USB runtime states seen                        : $(awk '{print $2}' "$OUT/usb.log" 2>/dev/null | sort | uniq -c | tr '\n' ' ')"
    echo
    if [ "$(n 'Link Key Request Neg|Authentication Failure|PIN or Key Missing')" != 0 ]; then
        echo "VERDICT: the pairing is refused (key) -> akmctl repair"
    elif [ -n "$BACK" ] && [ "$(n 'Conn(ect|ection) Request \(0x04\)')" != 0 ]; then
        echo "VERDICT: key-press reconnection works in this state (keyboard paged, host accepted)"
    elif [ -n "$BACK" ]; then
        echo "VERDICT: reconnected by the HOST paging (keyboard's own page not seen)"
    elif [ "$(n 'Conn(ect|ection) Request \(0x04\)')" = 0 ]; then
        echo "VERDICT: no Connection Request reached the host after the key press"
        echo "         (adapter suspended? $(grep -c suspended "$OUT/usb.log" 2>/dev/null || echo 0) suspended samples) -> apply udev/61-akm-bt-adapter-no-autosuspend.rules and retest"
    else
        echo "VERDICT: request seen but not completed -> read $T around 'Connection Request'"
    fi
} | tee "$OUT/summary.txt"
echo "files: $OUT"
