# apple-kb-monitor

Full telemetry, DDC/CI monitor control, and desktop integration for Apple Wireless Keyboards on Linux. Built by reverse-engineering the undocumented HID Feature Reports of the Broadcom BCM2042/BCM20733 Bluetooth controllers.

<!-- Badges -->
<!-- [![License: GPL-2.0-or-later](https://img.shields.io/badge/License-GPL--2.0--or--later-blue.svg)](LICENSE) -->

<!-- ![screenshot](docs/screenshot.png) -->

## What this project does

- Decodes **21 HID Feature Reports** from Apple keyboards: battery (3 methods), voltage, firmware, calibration curve, identity, BT parameters, config registers, ROM mirrors
- Provides a native **egui desktop GUI** (apihub-app) with 6 tabs for keyboard telemetry, monitor DDC/CI control, MQTT management, and system diagnostics
- Includes a standalone **DDC/CI CLI** (ddc-tool) with direct I2C access, 85 mapped VCPs, and sub-100ms latency -- no ddcutil dependency
- Maps all **13 special keys** at system level via keyd (Wayland-compatible)
- Handles **F1/F2 brightness** through raw evdev + DDC/CI writes + KDE OSD, with a circadian auto-brightness curve
- Integrates with **BlueZ Battery Provider** so battery percentage appears natively in KDE Plasma and GNOME
- Publishes to **MQTT / Home Assistant** with auto-discovery entities (battery, voltage, RSSI, monitor brightness)
- Reads **RSSI and TX power** via the BlueZ MGMT API (pure Rust, no C helper)
- Logs **battery history** to a JSONL append-only store for discharge analysis
- Includes original **reverse engineering documentation** for both the BCM2042 keyboard controller and the LG 34GN850 monitor scaler

## Components

| Binary | Language | LOC | Description |
|--------|----------|-----|-------------|
| `apihub-app` | Rust (egui) | 3030 | Desktop GUI: 6 tabs -- Keyboard, Display, Advanced, System, MQTT, Diagnostics |
| `ddc-tool` | Rust | 300 | CLI for direct I2C DDC/CI monitor read/write/json |
| `apple-kb-monitor` | Python | 2500 | Legacy CLI daemon (12 modes, systemd service, BlueZ Battery Provider) |

### apihub-app (main GUI)

The primary interface for daily use. Launches from the KDE application menu as "ApiHub".

- **Keyboard tab** -- battery (3 sources), voltage, ADC calibration, firmware, device identity, BT connection parameters, RSSI, TX power
- **Display tab** -- brightness, contrast, volume, input source, color preset, picture mode, sharpness, response time, FreeSync
- **Advanced tab** -- 85 VCPs including LG vendor registers, gamma, aspect ratio, smart energy saving, black stabilizer
- **System tab** -- DDC profiles (save/restore full monitor state), I2C bus auto-detection, circadian brightness
- **MQTT tab** -- broker/auth/entity configuration, start/stop bridge, connection status, last command received
- **Diagnostics tab** -- 15 system checks (binaries, services, hardware, config files, permissions)

### ddc-tool (CLI)

```bash
ddc-tool read 6 0x10        # read brightness from /dev/i2c-6
ddc-tool read 6 all         # read all 85 known VCPs
ddc-tool write 6 0x10 80    # set brightness to 80%
ddc-tool json 6             # JSON output of 30 essential VCPs (~5s)
```

Performance: write 30ms, read 100ms (vs ddcutil ~500ms/read). Uses `I2C_RDWR` ioctl with proper DDC/CI checksum validation.

### keyd key mapping

All 13 Apple special keys mapped at system level (Wayland-compatible):

| Key | Function | Method |
|-----|----------|--------|
| F1 | Monitor brightness down | DDC/CI + KDE OSD |
| F2 | Monitor brightness up | DDC/CI + KDE OSD |
| F3 | Overview (Expose) | keyd macro: Meta+Z |
| F4 | Grid View (Launchpad) | keyd macro: Meta+G |
| F5 | Lock Screen | keyd macro: Meta+L |
| F6 | Show Desktop | keyd macro: Meta+D |
| F7 | Previous Track | native (kernel) |
| F8 | Play / Pause | native (kernel) |
| F9 | Next Track | native (kernel) |
| F10 | Mute | native (kernel) |
| F11 | Volume Down | native (kernel) |
| F12 | Volume Up | native (kernel) |
| Eject | Eject | native (kernel) |

### KDE Plasma integration

- **Plasma widget** (`com.agenceapi.devicehub`) -- panel icon with compact representation and full popup (keyboard telemetry + monitor DDC controls)
- **Bluedevil panel patch** -- enriched Bluetooth panel showing battery %, firmware version, BT profiles, device class

## Quick start

### Arch Linux / Manjaro

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
sudo usermod -aG input $USER
# Log out and back in for group change
```

The PKGBUILD compiles two Rust binaries (`apihub-app` and `ddc-tool`) and installs all system integration files (udev, keyd, modprobe, desktop entry, Plasma widget).

### Launch

**apihub-app** is a graphical application, not a background service. Launch it from:
- KDE application menu: search for "ApiHub"
- Terminal: `apihub-app`

The legacy `apple-kb-monitor` CLI daemon can optionally run as a systemd user service for BlueZ Battery Provider and MQTT publishing:

```bash
systemctl --user enable --now apple-kb-monitor.service
```

### Configuration

Copy the example config and edit as needed:

```bash
mkdir -p ~/.config/apple-kb-monitor
cp /etc/apple-kb-monitor/config.toml.example ~/.config/apple-kb-monitor/config.toml
```

The config file controls DDC I2C bus path, MQTT broker credentials, monitor model, and brightness range. See [config.toml.example](config.toml.example) for all options.

## Module architecture

```
apihub-app (3030 LOC, 8 modules)
 +-- main.rs       (1477)  egui UI: 6 tabs, profile persistence, background polling
 +-- ddc.rs         (340)  DDC/CI I2C driver: auto-detect, read/write VCP, checksum
 +-- keyboard.rs    (330)  HID Feature Reports: 21 registers via ioctl
 +-- mqtt.rs        (240)  In-process MQTT client (rumqttc): HA auto-discovery
 +-- bluez.rs       (230)  BlueZ Battery Provider: zbus D-Bus, Battery1 interface
 +-- brightness.rs  (210)  F1/F2 evdev handler: raw input_event, DDC write, KDE OSD
 +-- rssi.rs        (137)  BlueZ MGMT API: AF_BLUETOOTH socket, opcode 0x0031
 +-- history.rs      (66)  JSONL battery history: append-only store

ddc-tool (300 LOC, 1 module)
 +-- main.rs        (300)  CLI: read/write/json, 85 VCP map, I2C_RDWR ioctl
```

### Data flow

```
Hardware                        apihub-app                      Desktop
+-----------------+    ioctl    +------------------+            +-----------+
| Apple Keyboard  |----------->| keyboard.rs      |            |           |
| /dev/hidrawN    |  HID feat  | 21 Feature Rpts  |   zbus     | KDE/GNOME |
|                 |            +------------------+----------->| Battery % |
| BCM2042 BT     |  AF_BT     | rssi.rs          |  Battery1  |           |
|                 |----------->| MGMT 0x0031      |            +-----------+
+-----------------+            +------------------+
                               | history.rs       |
+-----------------+    ioctl   | JSONL logging    |            +-----------+
| LG Monitor      |<--------->+------------------+  rumqttc   |           |
| /dev/i2c-N      |  I2C_RDWR | ddc.rs           |----------->| Home Asst |
| DDC/CI @ 0x37   |           | 85 VCPs          |  MQTT      | auto-disc |
+-----------------+            +------------------+            +-----------+
                    evdev      | brightness.rs    |  qdbus6    +-----------+
+-----------------+----------->| F1/F2 handler    |----------->| KDE OSD   |
| keyd            |  key event | circadian curve  |            |           |
| F3-F6 remapped  |            +------------------+            +-----------+
+-----------------+            | main.rs          |
                               | egui 6-tab UI    |
                               +------------------+
```

## Installed files

| Path | Description |
|------|-------------|
| `/usr/bin/apihub-app` | Desktop GUI (Rust egui) |
| `/usr/bin/ddc-tool` | DDC/CI monitor control CLI (Rust) |
| `/usr/bin/apple-kb-monitor` | Legacy CLI daemon (Python) |
| `/etc/apple-kb-monitor/config.toml.example` | Configuration template |
| `/usr/lib/systemd/user/apple-kb-monitor.service` | Battery Provider daemon (legacy CLI) |
| `/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules` | hidraw group permissions |
| `/etc/keyd/apple-keyboard.conf` | keyd special key mapping (05ac:0256) |
| `/etc/modprobe.d/hid_apple.conf` | fnmode=1 (media keys as default Fn row) |
| `/usr/share/applications/apihub-app.desktop` | KDE application launcher entry |
| `/usr/share/icons/hicolor/scalable/apps/apihub-scarab.svg` | Application icon |
| `/usr/share/plasma/plasmoids/com.agenceapi.devicehub/` | KDE Plasma widget |
| `/usr/share/apple-kb-monitor/kde/DeviceItem.qml` | Bluedevil panel patch |
| `/etc/dbus-1/system.d/com.agenceapi.AppleKbMonitor.conf` | D-Bus policy |

## Hardware support

### Supported keyboards

| Model | Controller | Status |
|-------|-----------|--------|
| Apple Wireless Keyboard A1314 (aluminum, ISO/ANSI/JIS) | BCM2042 | tested |
| Apple Wireless Keyboard A1016 (white) | BCM2042 | compatible |
| Apple Wireless Keyboard A1255 (aluminum) | BCM2042 | compatible |
| Apple Magic Keyboard A1644 | BCM20733 | untested (same register map) |
| Apple Magic Keyboard A2449 (Touch ID) | BCM20733 | untested |

### Tested environment

| Component | Version |
|-----------|---------|
| Kernel | 6.19.8 (Manjaro) |
| BlueZ | 5.86 |
| KDE Plasma | 6.x |
| keyd | 2.6.0 |
| Rust | stable (build only) |
| Monitor | LG 34GN850 (Realtek RTD2795 scaler) |
| GPU | NVIDIA (I2C pipeline aliasing handled) |

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

8 registers accept `HIDIOCSFEATURE` writes: `0x51`, `0x52`, `0x5A`, `0x60`, `0xF4`, `0xF5`, `0xF6`, `0xF7`. Not yet exploited -- requires BCM2042 flash/EEPROM reverse engineering to understand persistence and safety.

## Contributing

Contributions are welcome. This project values:

1. **Correctness** -- every HID register decode and DDC/CI VCP mapping has been verified against real hardware. New entries need evidence.
2. **No dependencies where unnecessary** -- DDC/CI uses raw I2C ioctl, RSSI uses raw AF_BLUETOOTH sockets, HID uses raw hidraw ioctl. No shelling out to ddcutil/bluetoothctl/hcitool.
3. **Performance** -- DDC reads complete in 100ms, writes in 30ms. The GUI renders at 60fps. Regressions in latency are bugs.

To contribute:

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor/apihub-app
cargo build
```

If you have a keyboard model not listed in the compatibility table, a `--dump` output from the legacy CLI or the raw HID report data from `apihub-app` is valuable for expanding hardware support.

## License

[GPL-2.0-or-later](LICENSE)

## Credits

Developed by [AgenceAPI](https://agenceapi.com).

The HID register map was built through original reverse engineering of the Broadcom BCM2042 controller firmware. The DDC/CI VCP map includes original reverse engineering of the LG 34GN850 scaler (Realtek RTD2795), documented in [docs/LG_34GN850_RE.md](docs/LG_34GN850_RE.md).

Built with [egui](https://github.com/emilk/egui), [zbus](https://gitlab.freedesktop.org/dbus/zbus), [rumqttc](https://github.com/bytebeamio/rumqtt).
