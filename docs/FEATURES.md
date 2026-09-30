# Features

Every item below was checked against the source (module in parentheses). Line counts: see [ARCHITECTURE.md](ARCHITECTURE.md).

## apihub-app (Rust, egui) -- primary interface

Single process, tray-first, 6 tabs (`main.rs`): Keyboard, Display, Advanced, System, MQTT, Diag.

| Area | Feature | Module |
|---|---|---|
| Keyboard | 21 HID Feature Reports via `HIDIOCGFEATURE` (battery precise, raw ADC voltage, calibration curve, firmware, build, name, identity, BT parameters) | `keyboard.rs` |
| Keyboard | 10 Apple models (6 BCM2042, 4 BCM20733) identified by PID | `keyboard.rs` (`APPLE_PIDS`) |
| Keyboard | Wake/connection events (HID Input Report 0x13), CapsLock/NumLock badges from sysfs | `keyboard.rs` |
| Keyboard | RSSI + TX power via BlueZ MGMT `GET_CONN_INFO` (0x0031); needs `CAP_NET_ADMIN`, returns nothing otherwise | `rssi.rs` |
| Keyboard | Battery history (JSONL), 24 h graph, discharge rate and time remaining | `history.rs`, `main.rs` |
| Desktop | BlueZ Battery Provider: exports `org.bluez.Battery1` so KDE/UPower show the keyboard battery | `bluez.rs` |
| Desktop | Tray icon (StatusNotifierItem + dbusmenu, pure zbus) | `tray.rs` |
| Display | DDC/CI over I2C (address 0x37), bus auto-detection, 4-tier smart polling (HOT/WARM/COLD/PBP) | `ddc.rs` |
| Display | Sliders for writable VCPs, picture modes, input source, factory resets (VCP 0x04/0x05/0x08) with confirmation | `main.rs`, `ddc.rs` |
| Display | PBP mirror registers when split mode is active | `ddc.rs`, `main.rs` |
| Display | F1/F2 brightness keys (evdev, keyd virtual keyboard) + KDE OSD via `qdbus6`, optional circadian curve | `brightness.rs` |
| Display | Named DDC profiles in `~/.config/apple-kb-monitor/profiles.json` | `main.rs` |
| Display | App Presets: picture mode switched from the active window class (KWin D-Bus scripting) | `main.rs` |
| Integration | MQTT client with Home Assistant auto-discovery (sensors, brightness/volume numbers, picture mode and input selects); discovery republished on reconnect | `mqtt.rs` |
| Diagnostics | Diag tab: binary, service, hardware, config and permission checks | `main.rs` |

## ddc-tool (Rust CLI)

```
ddc-tool read <bus> <vcp|all>
ddc-tool write <bus> <vcp> <value>
ddc-tool json <bus>
```

## Python CLI `apple-kb-monitor` (legacy, 2455 lines)

Modes: `--once`, `--status`, `--dump`, `--watch`, `--json`, `--waybar`, `--history`, `--graph`, `--export-csv`, `--metrics` (Prometheus), `--led NAME STATE`. Daemon options: `--threshold`, `--interval`, `--watch-interval`, `--no-provider`, `--mqtt HOST` (+ `--mqtt-port/-topic/-user/-pass`), `--auto-brightness`.

Also in the repository (not installed by the PKGBUILD): `mqtt-bridge.py` (brightness entity over MQTT), `apihub-settings` (PySide6 GUI), `rssi-helper.c`. See issue #16.

## System integration files

| File | Purpose |
|---|---|
| `udev/99-apple-kb-hidraw.rules` | `input` group access to Apple hidraw devices |
| `keyd/apple-keyboard.conf` | Apple special keys (device 05ac:0256) |
| `modprobe/hid_apple.conf` | `fnmode=1` |
| `dbus/com.agenceapi.AppleKbMonitor.conf` | Battery Provider registration policy |
| `systemd/apple-kb-monitor.service` | CLI daemon (user) |
| `plasma/com.agenceapi.devicehub/` | Plasma widget |
| `kde/DeviceItem.qml` | Bluedevil panel patch |

## Known limits

Tracked in Gitea issues: https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues
