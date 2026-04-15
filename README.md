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

- **962-line single Python script** — pure stdlib, zero external dependencies
- **77-line eBPF hook** for kernel-level HID tracing
- Systemd user service, udev rules, AUR package — ready for production use

## Features

| Feature | Source | Details |
|---|---|---|
| Precise battery % | HID `0xEA` | Pre-rounding value from ADC, before firmware quantization |
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
| RSSI / TX power | BlueZ MGMT | `GET_CONN_INFO` (opcode `0x0031`), requires `CAP_NET_ADMIN` |
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
| Apple Wireless Keyboard A1314 (aluminum, ISO) | BCM2042 | **Tested** |
| Apple Wireless Keyboard A1016 (white) | BCM2042 | Compatible |
| Apple Wireless Keyboard A1255 (aluminum) | BCM2042 | Compatible |
| Apple Magic Keyboard A1644 | BCM20733 | Compatible |
| Apple Magic Keyboard A2449 (Touch ID) | BCM20733 | Compatible |

All models share the same Broadcom HID register map.

## Installation

### AUR (Arch Linux)

```bash
yay -S apple-kb-monitor
```

### Manual

```bash
git clone https://github.com/litteulapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

## Setup

```bash
# Grant non-root hidraw access
sudo usermod -aG input $USER
# Log out and back in for group change to take effect

# Enable the background monitor
systemctl --user enable --now apple-kb-monitor.service
```

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

# RSSI (requires CAP_NET_ADMIN)
sudo apple-kb-monitor --status

# Daemon mode with low-battery notifications
apple-kb-monitor --threshold 15 --interval 300
```

## Permissions

| Feature | Requirement | Setup |
|---|---|---|
| All HID reports (battery, voltage, firmware, identity, etc.) | `input` group | `sudo usermod -aG input $USER` + re-login |
| RSSI, TX power | `CAP_NET_ADMIN` | Run with `sudo` |
| Desktop notifications | `libnotify` | `pacman -S libnotify` |

The udev rule (`99-apple-kb-hidraw.rules`) grants `input` group read/write access to Apple hidraw devices, including Bluetooth HID devices connected via uhid (matched by `DEVPATH` since uhid devices lack `idVendor`).

## Dependencies

All stdlib — no pip packages required.

| Dependency | Type | Purpose |
|---|---|---|
| `python` (>= 3) | Runtime | Core interpreter |
| `bluez` | Runtime | Bluetooth stack |
| `dbus` | Runtime | BlueZ device properties |
| `bluez-utils` | Optional | `bluetoothctl` for manual pairing |
| `libnotify` | Optional | `notify-send` for desktop notifications |

## How It Works

1. Discovers Apple Bluetooth keyboards via `/sys/class/hidraw/*/device/uevent`
2. Opens the `hidraw` device and sends `HIDIOCGFEATURE` ioctls for each known report ID
3. Decodes binary responses based on the reverse-engineered BCM2042 register map
4. Queries BlueZ for RSSI via MGMT socket (`GET_CONN_INFO`, opcode `0x0031`) and connection properties via D-Bus
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

RSSI is read via BlueZ MGMT `GET_CONN_INFO` (opcode `0x0031`), which triggers `HCI Read_RSSI` and `HCI Read_TX_Power` on the ACL connection handle. RSSI = 0 dBm is a valid measurement meaning optimal signal strength ("golden range"), not "unavailable".

## Project Structure

```
apple-kb-monitor            # Main script (962 lines, Python 3)
bpf/apple_kb_battery.bpf.c  # eBPF BPF hook (77 lines, C)
udev/99-apple-kb-hidraw.rules
systemd/apple-kb-monitor.service
PKGBUILD                    # Arch Linux / AUR package
```

## Acknowledgments

- BCM2042/BCM20733 HID Feature Report reverse engineering
- Linux HID subsystem (`hidraw`, `HIDIOCGFEATURE`)
- BlueZ MGMT interface
- Arch Linux packaging ecosystem

## License

[GPL-2.0-or-later](LICENSE)

## Author

Han — [AgenceAPI](https://github.com/litteulapi)
