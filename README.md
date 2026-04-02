# apple-kb-monitor

Full telemetry, key mapping, and desktop integration for Apple Wireless Keyboards on Linux. Built by reverse-engineering the undocumented HID Feature Reports of the Broadcom BCM2042/BCM20733 controllers.

This project also includes `ddc-tool`, a Rust binary for direct DDC/CI monitor control, and a complete keyd configuration for all 13 special keys.

## Components

### apple-kb-monitor (Python, 2448 LOC)

Async daemon and CLI for Apple Wireless Keyboard telemetry.

- **21 HID Feature Reports** decoded -- battery (3 methods), voltage, firmware, calibration curve, identity, BT parameters, config registers, ROM mirrors
- **BlueZ Battery Provider** -- Battery1 interface exposes precise battery % natively in KDE Plasma and GNOME
- **RSSI without sudo** -- `rssi-helper` C binary with `CAP_NET_ADMIN`, uses BlueZ MGMT API (opcode 0x0031)
- **SDP records parsed** -- full Bluetooth service profile enumeration via GetServiceRecords
- **LED control** -- read/write keyboard LEDs (Caps Lock, Num Lock)
- **Battery analytics** -- battery type detection (alkaline, NiMH, lithium), discharge rate, voltage interpolation, remaining time estimation
- **Wake event monitoring** -- HID Input Report 0x13 (vendor FF01) for device_ready and connection_request events
- **MQTT Home Assistant** -- auto-discovery entities (battery, voltage, RSSI) via MQTT
- **46 unit tests** -- full coverage of register decoding, voltage interpolation, analytics

#### 12 CLI modes

| Flag | Description |
|------|-------------|
| `--once` | Quick battery check (single read, exit) |
| `--status` | Full decoded report (all 21 registers + SDP + LEDs + analytics) |
| `--json` | JSON output (320+ data points) |
| `--waybar` | Waybar/polybar integration (JSON with tooltip) |
| `--dump` | Raw HID register hex dump |
| `--watch` | Live dashboard (terminal, auto-refresh) |
| `--graph` | Sparkline graphs (voltage, battery, RSSI) |
| `--export-csv` | Export history to CSV |
| `--metrics` | Prometheus metrics endpoint |
| `--history` | Battery/voltage history viewer |
| `--led` | LED control (`--led capslock on`) |
| `--auto-brightness` | Auto-brightness based on ambient conditions |

### ddc-tool (Rust, 288 LOC)

Direct I2C DDC/CI monitor control binary. No ddcutil dependency.

- **Direct I2C** -- uses `I2C_RDWR` ioctl for proper DDC/CI read/write
- **85 VCPs mapped** -- standard MCCS, LG vendor, undocumented, mirror registers
- **30 essential VCPs** for `json` mode (5s full state read)
- **Performance** -- Write: 30ms, Read: 100ms (vs ddcutil ~500ms/read)
- **3 modes** -- `read` (single or all), `write`, `json`

### keyd config

All 13 special keys mapped at system level (Wayland compatible):

| Key | Function | Method |
|-----|----------|--------|
| F1 | Monitor brightness down | DDC/CI + KDE OSD |
| F2 | Monitor brightness up | DDC/CI + KDE OSD |
| F3 | Overview (Expose) | keyd macro -> Meta+Z |
| F4 | Grid View (Launchpad) | keyd macro -> Meta+G |
| F5 | Lock Screen | keyd macro -> Meta+L |
| F6 | Show Desktop | keyd macro -> Meta+D |
| F7 | Previous Track | Native (kernel) |
| F8 | Play / Pause | Native (kernel) |
| F9 | Next Track | Native (kernel) |
| F10 | Mute | Native (kernel) |
| F11 | Volume Down | Native (kernel) |
| F12 | Volume Up | Native (kernel) |
| Eject | Eject | Native (kernel) |

### KDE integration

- **Plasma widget (ApiHub)** -- `com.agenceapi.devicehub` applet with compact representation (panel icon) and full popup (keyboard telemetry + monitor DDC controls)
- **Bluedevil panel patch** -- enriched Bluetooth panel showing battery %, firmware version, BT profiles, device class
- **apihub-settings** -- PySide6 desktop app for keyboard + monitor control (system tray, sliders, live data)

### Reverse engineering

- **LG 34GN850** -- 256 VCP scan, RAM dump via sidechannel 0xD1, firmware analysis. Full documentation in `docs/LG_34GN850_RE.md`
- **BCM2042** -- all HID Feature Report registers mapped, 8 writable registers identified, ADC calibration decoded

## Quick start

```bash
# Arch Linux / Manjaro
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
sudo usermod -aG input $USER
# Log out and back in, then:
systemctl --user enable --now apple-kb-monitor.service
systemctl --user enable --now apple-brightness.service
```

See `docs/INSTALL.md` for detailed installation instructions.

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

### ddc-tool usage

```bash
# Read a single VCP
ddc-tool read 6 0x10        # brightness

# Read all 85 known VCPs
ddc-tool read 6 all

# Write a VCP
ddc-tool write 6 0x10 80    # brightness to 80%

# JSON output (30 essential VCPs, ~5s)
ddc-tool json 6
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

## HID register map

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
| `0x5A` | **RW** | Discharge calibration curve (4 x u16 mV) |
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

### Writable registers (undocumented)

8 registers accept `HIDIOCSFEATURE` writes: 0x51, 0x52, 0x5A, 0x60, 0xF4, 0xF5, 0xF6, 0xF7. Not yet exploited -- needs BCM2042 flash/EEPROM reverse engineering to understand persistence and safety.

## Installed files

| File | Description |
|------|-------------|
| `/usr/bin/apple-kb-monitor` | Main daemon + CLI (Python) |
| `/usr/bin/apple-brightness-daemon` | F1/F2 DDC brightness + KDE OSD |
| `/usr/bin/apple-brightness-down` | DDC brightness -1% script |
| `/usr/bin/apple-brightness-up` | DDC brightness +1% script |
| `/usr/lib/apple-kb-monitor/rssi-helper` | RSSI binary (CAP_NET_ADMIN) |
| `/usr/lib/systemd/user/apple-kb-monitor.service` | Battery Provider daemon |
| `/usr/lib/systemd/user/apple-brightness.service` | Brightness daemon |
| `/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules` | hidraw permissions |
| `/etc/keyd/apple-keyboard.conf` | keyd special key mapping |
| `/etc/modprobe.d/hid_apple.conf` | fnmode=1 (media keys default) |
| `/usr/share/apple-kb-monitor/kde/DeviceItem.qml` | Bluedevil panel patch |

## Compatibility

| Component | Required | Tested |
|-----------|----------|--------|
| Kernel | >= 5.15 | 6.19.8 (Manjaro) |
| Python | >= 3.10 | 3.14.3 |
| BlueZ | >= 5.56 | 5.86 |
| Rust | >= 1.70 | (ddc-tool build only) |
| KDE Plasma | >= 6.0 | 6.x |
| keyd | >= 2.5 | 2.6.0 |

### Supported keyboards

- **Apple Wireless Keyboard A1314** (aluminum, ISO/ANSI/JIS) -- BCM2042 -- tested
- Apple Wireless Keyboard A1016 (white) -- BCM2042
- Apple Wireless Keyboard A1255 (aluminum) -- BCM2042
- Apple Magic Keyboard A1644 -- BCM20733 (same register map, untested)
- Apple Magic Keyboard A2449 (Touch ID) -- BCM20733 (untested)

## Project structure

```
apple-kb-monitor/
  apple-kb-monitor          Python daemon + CLI (2448 LOC)
  apihub-settings           PySide6 desktop app
  rssi-helper.c             RSSI via BlueZ MGMT API
  rssi-helper               Compiled C binary
  PKGBUILD                  Arch Linux package build
  apple-kb-monitor.install  Post-install hooks
  ddc-tool/
    Cargo.toml              Rust binary config
    src/main.rs             DDC/CI I2C implementation (288 LOC)
  keyd/
    apple-keyboard.conf     13 special keys (05ac:0256)
  modprobe/
    hid_apple.conf          fnmode=1
  systemd/
    apple-kb-monitor.service        User service (battery provider)
    apple-brightness.service        User service (F1/F2 brightness)
    apple-kb-monitor-system.service System service (alternative)
  udev/
    99-apple-kb-hidraw.rules        hidraw permissions for input group
  kde/
    DeviceItem.qml                  Bluedevil panel patch
    shortcuts/
      apple-brightness-daemon       F1/F2 evdev listener + DDC + OSD
      apple-brightness-down         DDC brightness -1%
      apple-brightness-up           DDC brightness +1%
  plasma/
    com.agenceapi.devicehub/        KDE Plasma widget
      metadata.json
      contents/ui/
        main.qml
        CompactRepresentation.qml
        FullRepresentation.qml
  icons/
    apihub-scarab.svg               Widget icon
  tests/
    test_apple_kb.py                46 unit tests
  docs/
    INSTALL.md                      Installation guide
    ARCHITECTURE.md                 System architecture
    LG_34GN850_RE.md                LG monitor reverse engineering
```

## License

GPL-2.0-or-later
