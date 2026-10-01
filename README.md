<p align="center">
  <h1 align="center">apple-kb-monitor</h1>
  <p align="center">
    Full telemetry monitor for Apple Wireless Keyboards on Linux.<br>
    Reads 21 undocumented HID Feature Reports from BCM2042/BCM20733 controllers.
  </p>
</p>

<p align="center">
  <a href="https://www.python.org/"><img src="https://img.shields.io/badge/Python-3-3776AB?logo=python&logoColor=white" alt="Python 3"></a>
  <a href="https://kernel.org/"><img src="https://img.shields.io/badge/Platform-Linux-FCC624?logo=linux&logoColor=black" alt="Linux"></a>
  <a href="https://aur.archlinux.org/packages/apple-kb-monitor"><img src="https://img.shields.io/badge/AUR-apple--kb--monitor-1793D1?logo=archlinux&logoColor=white" alt="AUR"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--2.0--or--later-blue" alt="GPL-2.0-or-later"></a>
</p>

---

## Overview

The Linux `hid-apple` driver only exposes basic battery percentage via the standard HID Battery Strength report (`0x47`). This tool goes far beyond that — it reads **21 undocumented Feature Reports** to extract precise battery levels, raw ADC voltage, firmware version, device identity, Bluetooth connection parameters, and more.

- **`apihub-app`** — Rust/egui desktop app (2 tabs: Keyboard and Diag, plus a tray icon): keyboard telemetry, battery history, BlueZ battery provider
- **`rssi-helper`** — tiny C helper carrying `cap_net_admin`, the only privileged binary (RSSI / TX power)
- **`apple-kb-monitor`** — legacy Python CLI (stdlib + dbus-fast): `--once`, `--json`, `--waybar`, `--metrics`…
- systemd user service, udev rule (`uaccess`), keyd config, Plasma widget, PKGBUILD, Gitea Actions CI

Documentation: [docs/](docs/) (FEATURES, CONFIGURATION, INSTALL, ARCHITECTURE, TROUBLESHOOTING, TESTING, CHANGELOG) and the [wiki](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/wiki). Roadmap: [milestones](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/milestones).

## Features

| Feature | Source | Details |
|---|---|---|
| Battery % (displayed) | Kernel `power_supply` | `/sys/class/power_supply/hid-<mac>-battery*`, the node UPower reads; source of truth |
| Precise battery % | HID `0xEA` | Pre-rounding value from ADC, diagnostics only |
| Raw ADC voltage | HID `0xF5` | 10-bit ADC, 3.3V reference |
| Calibration curve | HID `0x5A` | 4 discharge thresholds (mV) for 100/75/50/25% |
| Firmware version | HID `0x4F` | Chip firmware string |
| Build/revision | HID `0xFF` | Build number from controller ROM |
| Device name | HID `0x51-53` | 3 chunks read from chip ROM |
| Device identity | HID `0x4C` | 128-bit internal key |
| BT connection params | HID `0x46` | Interval + latency, live renegotiated values |
| BT supervision timeout | HID `0x49` | Link supervision timeout |
| Power management | HID `0x4A` | Controller power config |
| Device mode/class | HID `0x4B` | HID device class |
| Device state | HID `0x09` | 1=OK, 0=LOW |
| Config registers | HID `0xF6-F7` | Internal config |
| RSSI / TX power | BlueZ MGMT via `rssi-helper` | `GET_CONN_INFO` (opcode `0x0031`), helper has `cap_net_admin+ep` |
| Connection state | D-Bus | `org.bluez.Device1` properties |

## HID Report Map

| Report ID | Type | Description |
|---|---|---|
| `0x47` | Documented | Battery Strength (0-100%, same as `hid-apple` driver) |
| `0xEA` | Undocumented | Battery precise (pre-rounding ADC percentage) |
| `0xF5` | Undocumented | Battery voltage raw ADC (10-bit, 3.3V ref) |
| `0xF4` | Undocumented | ADC calibration reference |
| `0x5A` | Undocumented | Discharge curve (4 voltage thresholds) |
| `0x5B` | Undocumented | Voltage reference pair |
| `0x4F` | Undocumented | Firmware version |
| `0xFF` | Undocumented | Build/revision number |
| `0x51-53` | Undocumented | Device name string (3 ROM chunks) |
| `0x4C` | Undocumented | Device identity (128-bit) |
| `0x46` | Undocumented | BT connection interval + latency |
| `0x49` | Undocumented | BT supervision timeout |
| `0x4A` | Undocumented | Power management config |
| `0x4B` | Undocumented | Device mode/class |
| `0x09` | Documented | Device state flag |
| `0xF6-F7` | Undocumented | Config registers |

Reports discovered by brute-force scanning all 256 Feature Report IDs via `HIDIOCGFEATURE` ioctl.

## Hardware Compatibility

| Model | Controller | Status |
|---|---|---|
| Apple Wireless Keyboard A1314 (aluminum, ISO) | BCM2042 | **Tested** (fixtures in `tests/fixtures/a1314_iso/`) |
| A1255 (ANSI, ISO, JIS), A1314 2009 and aluminum (ANSI, ISO, JIS) | BCM2042 | In the model table, not tested |
| Magic Keyboard 2015 (A1644), with numeric keypad (A1843) | BCM20733 | In the model table, not tested |
| Magic Keyboard 2021 (A2450, A2449, A2520) and 2024 (3 variants) | BCM20733 / Apple | In the model table, not tested |

The 17 models come from `APPLE_MODELS` in `apihub-app/src/keyboard.rs` (PIDs from the kernel `hid-ids.h`; USB vendor `05AC` and Bluetooth vendor `004C` accepted). The raw undocumented HID reports are only read on the BCM2042 family; other models rely on the kernel battery.

## Installation

### AUR (Arch Linux)

```bash
yay -S apple-kb-monitor
```

### Manual

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

## Setup

No group membership is needed: the udev rule `70-apple-kb-hidraw.rules` tags Apple hidraw devices `uaccess`, so logind grants an ACL to the user of the active seat. Reconnect the keyboard (or `sudo udevadm trigger --subsystem-match=hidraw`) after installing.

Enable the daemon once (tray icon, low-battery notifications, history, Plasma widget data):

```bash
systemctl --user enable --now apple-kb-monitord.service
```

`apple-kb-monitord` is the single owner of the keyboard. The package does not enable it by itself: without this command it only starts on demand, when a client (`apihub-app`, the Plasma widget) calls `com.agenceapi.AppleKbMonitor1` on the session bus (D-Bus activation), so there is no tray icon nor low-battery alert until one of them has been opened.

The legacy Python service `apple-kb-monitor.service` does the same acquisition and conflicts with the daemon (`Conflicts=`): do **not** enable both. Enable it only if you do not want the Rust daemon.

## Usage

```bash
# Quick battery + voltage check
apple-kb-monitor --once
# Output: Apple Wireless Keyboard (A1314, aluminum, ISO)   100% (fine:98%)  2.981V

# Full decoded device report
apple-kb-monitor --status

# Raw Feature Report dump (for reverse engineering)
apple-kb-monitor --dump

# Live dashboard with auto-refresh
apple-kb-monitor --watch

# JSON output (for scripts, widgets, Home Assistant, etc.)
apple-kb-monitor --json

# Battery/voltage history log
apple-kb-monitor --history

# Daemon mode with low-battery notifications
apple-kb-monitor --threshold 15 --interval 300
```

## Permissions

| Feature | Requirement | Setup |
|---|---|---|
| Battery, HID reports (hidraw) | `uaccess` ACL (active seat user) | Installed by the package (`70-apple-kb-hidraw.rules`) |
| RSSI, TX power | `cap_net_admin+ep` on `/usr/lib/apple-kb-monitor/rssi-helper` | Applied by the package `post_install` (`setcap`); retry by hand if it prints a warning |
| Desktop notifications | `libnotify` | `pacman -S libnotify` |

The rule matches Bluetooth HID devices connected via uhid (`KERNELS` `0005:05AC:*` and `0005:004C:*`) and wired Apple keyboards (`0003:05AC:*`). It must sort before `73-seat-late.rules`, hence the `70-` prefix.

## Dependencies

`apihub-app` is a compiled Rust binary; the Python CLI needs only `python-dbus-fast` (see [docs/INSTALL.md](docs/INSTALL.md)).

| Dependency | Type | Purpose |
|---|---|---|
| `python`, `python-dbus-fast` | Runtime | Python CLI |
| `bluez` | Runtime | Bluetooth stack, Battery Provider API |
| `keyd` | Runtime | Apple special keys remapping |
| `bluez-utils` | Optional | `bluetoothctl` for manual pairing |
| `libnotify` | Optional | `notify-send` for low-battery notifications |
| `rust`, `gcc` | Build | `apihub-app`, `rssi-helper` |

## How It Works

1. Discovers Apple Bluetooth keyboards via `/sys/class/hidraw/*/device/uevent`
2. Reads the battery percentage from the kernel `power_supply` node of the keyboard (matched by MAC)
3. Opens the `hidraw` device and sends `HIDIOCGFEATURE` ioctls for each known report ID (BCM2042 family), decoded from the reverse-engineered register map
4. Runs `rssi-helper` for RSSI (BlueZ MGMT `GET_CONN_INFO`, opcode `0x0031`) and reads connection properties from BlueZ over D-Bus; exports the battery to BlueZ as `org.bluez.Battery1`
5. In daemon mode, logs readings to `$XDG_RUNTIME_DIR/apple-kb-monitor/history.jsonl` and sends desktop notifications via `notify-send` when battery drops below threshold

## Reverse Engineering Notes

The BCM2042 is a Broadcom single-chip Bluetooth HID controller:

- **ARM7TDMI** core
- **Bluetooth 2.0+EDR** radio
- **10-bit ADC** for battery voltage measurement
- Signed Apple firmware (not flashable from Linux)

The HID report descriptor only declares Reports `0x01` (keyboard), `0x47` (battery), `0x11-0x13` (consumer/vendor), and `0x09` (feature). All other reports (`0x46`, `0x49-0x53`, `0x5A-0x5B`, `0x60`, `0xEA-0xEB`, `0xF4-0xF7`, `0xFF`) are undocumented and were discovered by brute-force scanning all 256 Feature Report IDs.

### Discharge Calibration Curve

Report `0x5A` stores 4 voltage thresholds (millivolts) used by firmware to map ADC readings to battery percentage. Typical values for 2xAA alkaline:

```
100%  >= 2900 mV  (2.900 V)
 75%  >= 2450 mV  (2.450 V)
 50%  >= 2350 mV  (2.350 V)
 25%  >= 2000 mV  (2.000 V)
```

### RSSI

RSSI is read via BlueZ MGMT `GET_CONN_INFO` (opcode `0x0031`) by the `rssi-helper` binary (unprivileged sockets get status `0x14`), which triggers `HCI Read_RSSI` and `HCI Read_TX_Power` on the ACL connection handle. RSSI = 0 dBm is a valid measurement meaning optimal signal strength ("golden range"), not "unavailable".

## Project Structure

```
apihub-app/                 # Rust/egui GUI (src/: main, keyboard, power, bluez, rssi, history, tray)
rssi-helper.c               # C helper with cap_net_admin (RSSI / TX power)
apple-kb-monitor            # Python CLI
apihub-settings             # PySide6 settings window (legacy)
udev/ systemd/ keyd/ modprobe/ dbus/             # system integration
plasma/ kde/                # Plasma widget, Bluedevil panel patch
tests/                      # pytest, fixtures/ (real A1314 capture), live/check_keyboard.sh
.gitea/workflows/ci.yml     # CI
docs/                       # documentation
PKGBUILD, .SRCINFO, apple-kb-monitor.install   # Arch Linux package
```

## Acknowledgments

- BCM2042/BCM20733 HID Feature Report reverse engineering
- Linux HID subsystem (`hidraw`, `HIDIOCGFEATURE`)
- BlueZ MGMT interface
- Arch Linux packaging ecosystem

## License

[GPL-2.0-or-later](LICENSE)

## Author

Han — [AgenceAPI](https://gitea.pika.agenceapi.fr/adminapi)
