# Fixtures clavier (sans matériel)

## `a1314_iso/` — CAPTURÉ sur le matériel réel (2026-10-01, lecture seule, sans sudo)

Apple Wireless Keyboard A1314 ISO « Clavier de maria #1 », `04:DB:56:CA:42:EE`, BT, `0005:05AC:0256`.
Chemins sysfs d'origine entre parenthèses.

| Fichier | Source |
|---|---|
| `report_descriptor.bin` / `.hex` | `/sys/class/hidraw/hidraw7/device/report_descriptor` (224 o) |
| `hid_device.uevent` | `.../device/uevent` (HID_ID, HID_UNIQ, DRIVER=apple) |
| `hidraw.uevent` | `/sys/class/hidraw/hidraw7/uevent` |
| `power_supply.uevent`, `ps_*` | `/sys/class/power_supply/hid-04:db:56:ca:42:ee-battery-71/` (capacité 90, Discharging) |
| `bluetoothctl_info.txt` | `bluetoothctl info 04:DB:56:CA:42:EE` |
| `upower_info.txt` | `upower -i .../battery_hid_04odbo56ocao42oee_battery_71` |
| `udevadm_hidraw.props` | `udevadm info -q property /dev/hidraw7` |
| `input/` | attributs `name phys uniq id/* capabilities/{ev,key,led}` de l'input evdev |

Le nom du power_supply porte un suffixe `-71` (report id) : `hid-<mac>-battery-71`, pas `hid-<mac>-battery`.

## `synthetic/` — SYNTHÉTIQUE (non capturé)

Les GET_REPORT bruts (`HIDIOCGFEATURE`) exigent d'ouvrir `/dev/hidraw7`, en `0600 root` sur ce poste
(règle udev `uaccess` non installée) : illisibles sans sudo, donc non capturés.

* `get_feature_0x47_battery90.hex` : 16 o, `0x47` (Battery Strength) + valeur `0x5a` = 90 %, reste à 0.
  Forme reconstruite d'après `hid_get_feature()` / `read_all_reports()` dans `apple-kb-monitor` ; non vérifiée sur le fil.

## Usage

```bash
python3 - <<'PY'
import pathlib; d = pathlib.Path("tests/fixtures/a1314_iso")
print(dict(l.split("=",1) for l in (d/"hid_device.uevent").read_text().splitlines()))
PY
```
