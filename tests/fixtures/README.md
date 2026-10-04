# Keyboard fixtures (no hardware)

## `a1314_iso/`: CAPTURED on the real hardware (2026-10-01, read only, no sudo)

Apple Wireless Keyboard A1314 ISO named "Clavier de alice #1" (the name stored in the keyboard), `AA:BB:CC:DD:EE:F1`, BT, `0005:05AC:0256`.
Original sysfs paths in brackets.

| File | Source |
|---|---|
| `report_descriptor.bin` / `.hex` | `/sys/class/hidraw/hidraw7/device/report_descriptor` (224 B) |
| `hid_device.uevent` | `.../device/uevent` (HID_ID, HID_UNIQ, DRIVER=apple) |
| `hidraw.uevent` | `/sys/class/hidraw/hidraw7/uevent` |
| `power_supply.uevent`, `ps_*` | `/sys/class/power_supply/hid-aa:bb:cc:dd:ee:f1-battery-71/` (capacity 90, Discharging) |
| `bluetoothctl_info.txt` | `bluetoothctl info AA:BB:CC:DD:EE:F1` |
| `input/` | attributes `name phys uniq id/* capabilities/{ev,key,led}` of the evdev input |

The power_supply name carries a `-71` suffix (report id): `hid-<mac>-battery-71`, not `hid-<mac>-battery`.

## `synthetic/`: SYNTHETIC (not captured)

Raw GET_REPORTs (`HIDIOCGFEATURE`) need `/dev/hidraw7` open, `0600 root` on that machine
(udev `uaccess` rule not installed): unreadable without sudo, so not captured.

* `get_feature_0x47_battery90.hex`: 16 B, `0x47` (Battery Strength) + value `0x5a` = 90 %, the rest 0.
  Shape rebuilt from `hid_get_feature()` / `read_all_reports()` in `apple-kb-monitor`; not checked on the wire.

## Usage

```bash
python3 - <<'PY'
import pathlib; d = pathlib.Path("tests/fixtures/a1314_iso")
print(dict(l.split("=",1) for l in (d/"hid_device.uevent").read_text().splitlines()))
PY
```
