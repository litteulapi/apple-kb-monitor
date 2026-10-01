#!/usr/bin/env bash
# End-to-end tests of the apihub-app window under Xvfb, with a simulated
# keyboard. Never touches the real keyboard, needs no root.
#
#   tests/e2e/run.sh [--out DIR] [--only a,b] [--duration S] [--update-refs]
#                    [--bin-dir DIR]   test prebuilt binaries (apihub-app,
#                                      apple-kb-monitord) instead of building
#
# Scenarios (tests/e2e/e2e.py): responsive, open_close, daemon_absent,
# daemon_slow, daemon_dies, daemon_garbage, history_50k, history_corrupt,
# resize, spy, screens. Exit 0 = all passed, 1 = one failed (message +
# capture path printed, details in DIR/<scenario>/result.json), 77 = a tool
# is missing (skipped, never a silent pass).
#
# Isolation (bubblewrap, unprivileged user namespace):
#   /sys/class/hidraw, /sys/bus/hid/devices, /sys/class/power_supply -> fake
#   trees built from tests/fixtures/a1314_iso (anonymised MAC);
#   /dev -> minimal, /dev/hidraw7 = empty regular file (the keyboard: any
#   write makes it grow), /dev/hidraw3 = a fake mouse (must never be opened);
#   /usr/lib/apple-kb-monitor (rssi-helper, akm-helper) hidden, pkexec = false;
#   private session bus + private "system" bus with a fake BlueZ/UPower.
set -uo pipefail
export LC_ALL=C

top=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
out="" only="" duration=60 update="" bindir=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift ;;
    --only) only=$2; shift ;;
    --duration) duration=$2; shift ;;
    --update-refs) update=--update-refs ;;
    --bin-dir) bindir=$2; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "usage: $0 [--out DIR] [--only a,b] [--duration S] [--update-refs] [--bin-dir DIR]" >&2; exit 64 ;;
  esac
  shift
done
out=${out:-$top/scripts/out/e2e-$(date -u +%Y%m%dT%H%M%SZ)}
mkdir -p "$out"

missing=""
for b in Xvfb xdotool import bwrap dbus-daemon busctl strace python3; do
  command -v "$b" >/dev/null 2>&1 || missing="$missing $b"
done
python3 -c 'import gi; gi.require_version("Gio","2.0"); from gi.repository import Gio; import PIL' 2>/dev/null || missing="$missing python-gobject/python-pillow"
if [ -n "$missing" ]; then echo "e2e skipped: missing$missing"; exit 77; fi
bwrap --dev-bind / / --tmpfs /sys/class/hidraw true 2>/dev/null || { echo "e2e skipped: bwrap cannot mount over /sys (user namespaces disabled?)"; exit 77; }

if [ -z "$bindir" ]; then
  echo "building apihub-app + apple-kb-monitord (release)..."
  (cd "$top/apihub-app" && cargo build --locked --release -p apihub-app -p apple-kb-monitord 2>&1 | tail -3) || exit 1
  bindir="${CARGO_TARGET_DIR:-$top/apihub-app/target}/release"
fi
for b in apihub-app apple-kb-monitord; do
  [ -x "$bindir/$b" ] || { echo "missing binary $bindir/$b"; exit 1; }
done

work=$(mktemp -d "${TMPDIR:-/tmp}/akm-e2e.XXXXXX")
trap 'rm -rf "$work"' EXIT
fx="$top/tests/fixtures/a1314_iso"
mac=04:db:56:00:e2:e0
anon() { sed -e "s/04:db:56:ca:42:ee/$mac/Ig" -e 's/Clavier de maria #1/Clavier de test/' -e 's/6c:94:66:52:7c:0d/00:1a:7d:00:00:01/I' "$1"; }

# Fake sysfs: the keyboard (hidraw7) and a mouse (hidraw3, never to be opened).
s="$work/sys"
mkdir -p "$s/class/hidraw/hidraw7/device" "$s/class/hidraw/hidraw3/device" \
         "$s/bus/hid/devices/0005:05AC:0256.0007/power_supply/hid-$mac-battery-71" \
         "$s/bus/hid/devices/0003:046D:C077.0003" "$s/class/power_supply/hid-$mac-battery-71"
anon "$fx/hid_device.uevent" > "$s/class/hidraw/hidraw7/device/uevent"
cp "$fx/report_descriptor.bin" "$s/class/hidraw/hidraw7/device/report_descriptor"
cp "$fx/hidraw.uevent" "$s/class/hidraw/hidraw7/uevent"
anon "$fx/hid_device.uevent" > "$s/bus/hid/devices/0005:05AC:0256.0007/uevent"
printf 'DRIVER=hid-generic\nHID_ID=0003:0000046D:0000C077\nHID_NAME=Logitech USB Optical Mouse\nHID_PHYS=usb-0000:00:14.0-2/input0\nHID_UNIQ=\n' \
  | tee "$s/class/hidraw/hidraw3/device/uevent" > "$s/bus/hid/devices/0003:046D:C077.0003/uevent"
printf 'MAJOR=243\nMINOR=3\nDEVNAME=hidraw3\n' > "$s/class/hidraw/hidraw3/uevent"
ps="$s/class/power_supply/hid-$mac-battery-71"
anon "$fx/power_supply.uevent" > "$ps/uevent"
for f in capacity model_name online present scope status type; do anon "$fx/ps_$f" > "$ps/$f"; done
: > "$work/hidraw7"; : > "$work/hidraw3"
chmod 600 "$work/hidraw7" "$work/hidraw3"

python3 "$top/tests/e2e/gen_history.py" seed "$work/history-seed.jsonl"
python3 "$top/tests/e2e/gen_history.py" big "$work/history-50k.jsonl"
python3 "$top/tests/e2e/gen_history.py" bad "$work/history-bad.jsonl"

hide=()
# Xvfb (GLX/glamor) crashes without the DRM nodes; they are the GPU, not HID.
[ -d /dev/dri ] && hide+=(--dev-bind /dev/dri /dev/dri)
[ -d /usr/lib/apple-kb-monitor ] && hide+=(--tmpfs /usr/lib/apple-kb-monitor)
[ -e /usr/bin/pkexec ] && hide+=(--ro-bind /usr/bin/false /usr/bin/pkexec)
echo "e2e: binaries from $bindir, results in $out"
bwrap --dev-bind / / --dev /dev --tmpfs /dev/shm --proc /proc \
  --bind "$s/class/hidraw" /sys/class/hidraw \
  --bind "$s/bus/hid/devices" /sys/bus/hid/devices \
  --bind "$s/class/power_supply" /sys/class/power_supply \
  --bind "$work/hidraw7" /dev/hidraw7 --bind "$work/hidraw3" /dev/hidraw3 \
  "${hide[@]}" \
  --unshare-pid --die-with-parent --setenv AKM_E2E 1 \
  python3 "$top/tests/e2e/e2e.py" --work "$work" --out "$out" \
    --app "$bindir/apihub-app" --daemon "$bindir/apple-kb-monitord" \
    --seed-history "$work/history-seed.jsonl" --big-history "$work/history-50k.jsonl" \
    --bad-history "$work/history-bad.jsonl" --duration "$duration" \
    ${only:+--only "$only"} $update
rc=$?
echo "e2e details: $out/summary.json"
exit $rc
