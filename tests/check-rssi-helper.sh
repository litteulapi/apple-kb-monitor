#!/bin/sh
# rssi-helper answers only for a connected Apple keyboard: built against a
# fake /sys/bus/hid/devices, every other device or argument is refused.
set -eu
top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }
w=$(mktemp -d "${TMPDIR:-/tmp}/akm-rssi.XXXXXX")
trap 'rm -rf "$w"' EXIT
hid="$w/hid"
dev() { mkdir -p "$hid/$1"; printf 'HID_ID=x\nHID_NAME=%s\nHID_UNIQ=%s\n' "${3:-kb}" "$2" > "$hid/$1/uevent"; }
dev 0005:05AC:0256.0001 aa:bb:cc:dd:ee:01
dev 0005:004C:0267.0002 aa:bb:cc:dd:ee:02
dev 0005:05AC:030D.0003 aa:bb:cc:dd:ee:03
dev 0005:0A12:0001.0004 aa:bb:cc:dd:ee:04
dev 0003:05AC:0256.0005 aa:bb:cc:dd:ee:05
dev 0005:05AC:0256.0006 00:00:00:00:00:00 aa:bb:cc:dd:ee:06
dev 0005:05AC:0256.0007 "aa:bb:cc:dd:ee:07 "
gcc -Wall -Wextra -Werror -DAKM_HID_ROOT="\"$hid\"" -o "$w/rssi-helper" "$top/rssi-helper.c"
run() { set +e; "$w/rssi-helper" "$@" >/dev/null 2>&1; rc=$?; set -e; echo $rc; }
for m in aa:bb:cc:dd:ee:01 AA:BB:CC:DD:EE:01 aa:bb:cc:dd:ee:02; do
  rc=$(run "$m"); [ "$rc" != 6 ] && [ "$rc" != 1 ] || fail "Apple keyboard $m refused (exit $rc)"
done
for m in aa:bb:cc:dd:ee:03 aa:bb:cc:dd:ee:04 aa:bb:cc:dd:ee:05 aa:bb:cc:dd:ee:06 aa:bb:cc:dd:ee:07 11:22:33:44:55:66; do
  [ "$(run "$m")" = 6 ] || fail "$m must be refused (not a connected Apple keyboard)"
done
for a in "" "../../etc/passwd" "aa:bb:cc:dd:ee:1" "aa:bb:cc:dd:ee:011" "aa-bb-cc-dd-ee-01" "aa:bb:cc:dd:ee:0g"; do
  [ "$(run "$a")" = 1 ] || fail "argument '$a' must be a usage error"
done
[ "$(run)" = 1 ] || fail "no argument must be a usage error"
[ "$(run aa:bb:cc:dd:ee:01 0 x)" = 1 ] || fail "extra argument must be a usage error"
[ "$(run aa:bb:cc:dd:ee:01 -1)" = 1 ] || fail "bad controller index must be a usage error"
# the production binary reads the real sysfs only, no environment
grep -q '#define AKM_HID_ROOT "/sys/bus/hid/devices"' "$top/rssi-helper.c" || fail "default HID root"
if grep -nE 'getenv|secure_getenv|environ' "$top/rssi-helper.c"; then fail "rssi-helper must read no environment"; fi
# MGMT status codes from include/net/bluetooth/mgmt.h
grep -q '0x02 ? " (not connected)"' "$top/rssi-helper.c" || fail "MGMT 0x02 is NOT_CONNECTED"
grep -q '0x0d ? " (invalid parameters)"' "$top/rssi-helper.c" || fail "MGMT 0x0d is INVALID_PARAMS"
grep -q 'if (setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO' "$top/rssi-helper.c" || fail "SO_RCVTIMEO result must be checked"
# same closed list of keyboards as the udev rule
u=$(grep -o '0005:05AC:[0-9A-F]\{4\}' "$top/udev/70-apple-kb-hidraw.rules" | cut -d: -f3 | sort -u | tr '\n' ' ')
c=$(sed -n '/KB_PIDS\[\] = {/,/};/p' "$top/rssi-helper.c" | grep -o '"[0-9A-F]\{4\}"' | tr -d '"' | sort -u | tr '\n' ' ')
[ "$u" = "$c" ] || fail "rssi-helper keyboard list differs from the udev rule: [$c] vs [$u]"
echo "PASS rssi-helper"
