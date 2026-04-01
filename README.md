# apple-kb-monitor

Full telemetry, key mapping, and desktop integration for Apple Wireless Keyboards on Linux. Built by reverse-engineering the undocumented HID Feature Reports of the Broadcom BCM2042/BCM20733 controllers.

## Features

- **21 HID Feature Reports** decoded — battery (3 methods), voltage, firmware, calibration curve, identity, BT parameters, config registers, ROM mirrors
- **BlueZ Battery Provider** — precise battery % appears natively in KDE Plasma / GNOME
- **DDC/CI monitor brightness** — F1/F2 control external monitor brightness with KDE OSD
- **keyd integration** — all 13 special keys mapped at system level (Wayland compatible)
- **RSSI without sudo** — C helper binary with `CAP_NET_ADMIN`
- **SDP service records** — full Bluetooth profile parsing
- **LED control** — read/write keyboard LEDs
- **Analytics** — battery type detection, discharge rate, remaining time estimation
- **KDE Bluedevil patch** — enriched Bluetooth panel (battery, firmware, profiles, BT class)
- **12 output modes** — CLI, JSON, CSV, Prometheus, Waybar, sparkline graphs

## Install (Arch Linux / Manjaro)

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
sudo usermod -aG input $USER
# Log out and back in, then:
systemctl --user enable --now apple-kb-monitor.service
systemctl --user enable --now apple-brightness.service
```

### What gets installed

| File | Description |
|------|-------------|
| `/usr/bin/apple-kb-monitor` | Main daemon + CLI (Python) |
| `/usr/bin/apple-brightness-daemon` | F1/F2 DDC brightness + OSD |
| `/usr/bin/apple-brightness-down` | DDC brightness -1% script |
| `/usr/bin/apple-brightness-up` | DDC brightness +1% script |
| `/usr/lib/apple-kb-monitor/rssi-helper` | RSSI binary (CAP_NET_ADMIN) |
| `/usr/lib/systemd/user/apple-kb-monitor.service` | Battery Provider daemon |
| `/usr/lib/systemd/user/apple-brightness.service` | Brightness daemon |
| `/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules` | hidraw permissions |
| `/etc/keyd/apple-keyboard.conf` | keyd special key mapping |
| `/etc/modprobe.d/hid_apple.conf` | fnmode=1 (media keys default) |

### Dependencies

- `python`, `python-dbus-fast`, `python-dbus` — daemon
- `bluez` (>= 5.56) — Battery Provider API
- `keyd` — system-level key remapping
- `ddcutil` — DDC/CI monitor brightness

## Special Keys

All 13 special keys are functional via `keyd` (system-level, Wayland compatible):

| Key | Function | Method |
|-----|----------|--------|
| F1 | Monitor brightness down | DDC/CI + KDE OSD |
| F2 | Monitor brightness up | DDC/CI + KDE OSD |
| F3 | Overview (Exposé) | keyd macro → Meta+W |
| F4 | Grid View (Launchpad) | keyd macro → Meta+G |
| F5 | Lock Screen | keyd macro → Meta+L |
| F6 | Show Desktop | keyd macro → Meta+D |
| F7 | Previous Track | Native (kernel) |
| F8 | Play / Pause | Native (kernel) |
| F9 | Next Track | Native (kernel) |
| F10 | Mute | Native (kernel) |
| F11 | Volume Down | Native (kernel) |
| F12 | Volume Up | Native (kernel) |
| Eject | Eject | Native (kernel) |

## Usage

```bash
# Quick battery check
apple-kb-monitor --once

# Full decoded report (all 21 registers + SDP + LEDs + analytics)
apple-kb-monitor --status

# JSON (320+ data points)
apple-kb-monitor --json

# Raw HID register dump
apple-kb-monitor --dump

# Live dashboard
apple-kb-monitor --watch

# Waybar/polybar integration
apple-kb-monitor --waybar

# Battery/voltage history
apple-kb-monitor --history

# Sparkline graphs (voltage, battery, RSSI)
apple-kb-monitor --graph

# Export CSV
apple-kb-monitor --export-csv

# Prometheus metrics
apple-kb-monitor --metrics

# LED control
apple-kb-monitor --led capslock on

# Daemon with MQTT Home Assistant
apple-kb-monitor --mqtt 192.168.8.10

# Daemon (systemd service)
apple-kb-monitor --threshold 15 --interval 300
```

### Waybar config

```json
"custom/apple-kb": {
    "exec": "apple-kb-monitor --waybar",
    "return-type": "json",
    "interval": 60
}
```

### MQTT Home Assistant

```bash
apple-kb-monitor --mqtt 192.168.8.10 --mqtt-port 1883 --mqtt-topic homeassistant
```

Creates auto-discovery entities: `sensor.apple_kb_battery`, `sensor.apple_kb_voltage`, `sensor.apple_kb_rssi`.

## HID Register Map

### Feature Reports (21/21 decoded)

| Report | RW | Description |
|--------|-----|-------------|
| `0x09` | RO | Device state flag (1=OK, 0=LOW) |
| `0x46` | RO | BT connection interval + latency |
| `0x47` | RO | Battery % standard (rounded by firmware) |
| `0x49` | RO | BT supervision timeout |
| `0x4A` | RO | Power management config |
| `0x4B` | RO | Device mode + class |
| `0x4C` | RO | Identity key (144-bit) |
| `0x4F` | RO | Firmware version (major.minor) |
| `0x51` | **RW** | Device name chunk 1 (8 bytes) |
| `0x52` | **RW** | Device name chunk 2 (8 bytes) |
| `0x53` | RO | Device name chunk 3 (locked) |
| `0x5A` | **RW** | Discharge calibration curve (4 × u16 mV) |
| `0x5B` | RO | Voltage reference pair |
| `0x60` | **RW** | ROM mirror of 0x5A |
| `0xEA` | RO | Battery % precise (pre-rounding ADC value) |
| `0xEB` | RO | ROM mirror of 0x5A (locked) |
| `0xF4` | **RW** | ADC calibration reference |
| `0xF5` | **RW** | ADC raw voltage (10-bit, 3.3V ref) |
| `0xF6` | **RW** | Config register 1 |
| `0xF7` | **RW** | Config register 2 |
| `0xFF` | RO | Firmware build/revision number |

### Input Reports

| Report | Description |
|--------|-------------|
| `0x01` | Keyboard scancodes (kernel hid-apple) |
| `0x11` | Consumer: Eject |
| `0x12` | Consumer: Play/Pause, Next, Prev, FF, Rew |
| `0x13` | Vendor FF01: wake signal (device_ready, connection_request) |

### Writable Registers (undocumented)

8 registers accept `HIDIOCSFEATURE` writes. **Not yet exploited** — needs BCM2042 flash/EEPROM reverse engineering to understand persistence and safety.

## Compatibility

| Component | Required | Tested |
|-----------|----------|--------|
| Kernel | >= 5.15 | 6.19.8 (Manjaro) |
| Python | >= 3.10 | 3.14.3 |
| BlueZ | >= 5.56 | 5.86 |
| KDE Plasma | >= 6.0 | 6.x |
| keyd | >= 2.5 | 2.6.0 |
| ddcutil | >= 1.0 | installed |

### Supported keyboards

- **Apple Wireless Keyboard A1314** (aluminum, ISO/ANSI/JIS) — BCM2042 ✅ tested
- Apple Wireless Keyboard A1016 (white) — BCM2042
- Apple Wireless Keyboard A1255 (aluminum) — BCM2042
- Apple Magic Keyboard A1644 — BCM20733 (same register map, untested)
- Apple Magic Keyboard A2449 (Touch ID) — BCM20733 (untested)

## Architecture

```
apple-kb-monitor daemon (async, dbus-fast)
├── HID Feature Reports (hidraw ioctl) → 21 registers decoded
├── HID Input Report 0x13 (HidrawMonitor) → wake/connection events
├── BlueZ Battery Provider (BatteryProvider1) → KDE/GNOME battery
├── BlueZ D-Bus signals (PropertiesChanged, InterfacesAdded)
├── BlueZ MGMT (rssi-helper CAP_NET_ADMIN) → RSSI, TX Power
├── BlueZ SDP (GetServiceRecords) → 2 service records parsed
└── History logging (JSONL) + notifications (notify-send)

apple-brightness-daemon (evdev → keyd virtual keyboard)
├── F1/F2 → ddcutil setvcp (DDC/CI)
└── KDE OSD via brightnessChanged D-Bus

keyd (/etc/keyd/apple-keyboard.conf)
├── F3 → Overview, F4 → Grid View
├── F5 → Lock, F6 → Show Desktop
└── F1/F2 passthrough to brightness daemon
```

## License

GPL-2.0-or-later
