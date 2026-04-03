# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [3.0.0] - 2025-04-03

Full Rust rewrite. The Python CLI daemon remains for backward compatibility but the primary interface is now `apihub-app`, a native egui desktop application.

### Added
- **apihub-app**: native Rust GUI (egui) with 6 tabs -- Keyboard, Display, Advanced, System, MQTT, Diagnostics
- **8-module architecture**: main, ddc, keyboard, bluez, brightness, mqtt, rssi, history
- **Pure Rust RSSI reader**: replaces the C `rssi-helper` binary -- uses AF_BLUETOOTH + MGMT opcode 0x0031 directly
- **In-process MQTT client**: rumqttc replaces the Python `mqtt-bridge.py` subprocess
- **BlueZ Battery Provider**: zbus D-Bus integration, Battery1 interface for native KDE/GNOME battery display
- **F1/F2 brightness handler**: raw evdev listener with DDC/CI write and KDE OSD notification, replaces `apple-brightness-daemon` shell script
- **Circadian auto-brightness**: time-of-day brightness curve (30% night, 70% day, smooth ramps)
- **DDC profiles**: save and restore full monitor state (all VCP values) from the GUI
- **I2C bus auto-detection**: probes all `/dev/i2c-*` for DDC-capable displays
- **Battery history**: JSONL append-only store with per-reading timestamps
- **Desktop notifications**: notify-rust for low battery alerts
- **config.toml**: centralized configuration for DDC bus, MQTT credentials, brightness range
- **.desktop entry**: searchable as "ApiHub" in KDE application menu with scarab icon
- **Diagnostics tab**: 15 automated system checks (binaries, services, hardware, config, permissions)
- **D-Bus policy**: `com.agenceapi.AppleKbMonitor.conf` for BlueZ Battery Provider access

### Changed
- **PKGBUILD**: builds 2 Rust binaries via cargo, slimmed dependencies (only `bluez` and `keyd` required)
- **DDC/CI driver**: combined `I2C_RDWR` 2-message ioctl for zero-sleep reads, NVIDIA pipeline aliasing workaround
- **DDC read performance**: 100ms per VCP (was ~500ms with ddcutil), 3-tier polling with 710ms/cycle
- **VCP map**: brute-force verified all Advanced VCPs against LG 34GN850 hardware

### Removed
- `apihub-settings` (PySide6 desktop app) -- replaced by apihub-app
- `rssi-helper` (C binary with CAP_NET_ADMIN) -- replaced by pure Rust in rssi.rs
- `mqtt-bridge.py` (Python MQTT daemon) -- replaced by in-process rumqttc
- `apple-brightness-daemon` (shell script) -- replaced by brightness.rs
- `apple-brightness-down` / `apple-brightness-up` (helper scripts) -- replaced by brightness.rs
- `apple-brightness.service` (systemd) -- brightness handled inside apihub-app
- `mqtt-bridge.service` (systemd) -- MQTT handled inside apihub-app

## [2.5.0] - 2025-04-01

### Added
- ddc-tool expanded to 85 VCPs with all data-bearing registers mapped
- LG 34GN850 full reverse engineering documentation (VCP map, OSD analysis, mirror registers, vendor codes)
- Scaler RAM map via sidechannel VCP 0xD1 -- 5 regions decoded as VCP response cache ring buffers
- VCP 0x72 (MCCS Gamma) discovered as writable, bypasses picture mode gamma lock on 0xFE
- LG OnScreen Control SDK analysis (DDC/CI VCP Set/Get, no secret commands)
- KDE OSD for brightness changes (LG firmware has no native OSD for DDC brightness writes)

### Changed
- Complete documentation rewrite for v2.5

## [2.4.0] - 2025-03-31

### Added
- keyd configuration for all 13 Apple special keys (Wayland-compatible)
- DDC/CI brightness daemon (F1/F2 via evdev + ddc-tool + KDE OSD)
- Complete PKGBUILD installer with post-install hooks
- MQTT Home Assistant integration with auto-discovery entities
- 46 unit tests for register decoding, voltage interpolation, analytics
- Compatibility matrix documentation

## [2.3.0] - 2025-03-31

### Added
- KDE Bluedevil enhanced panel (patched DeviceItem.qml with battery %, firmware, profiles, device class)
- Full BlueZ property enumeration (zero exceptions)
- Complete kernel/BlueZ/UPower/KDE stack mapping

## [2.2.0] - 2025-03-31

### Added
- SDP service record parsing (HID, PnP, SPP profiles)
- LED control (Caps Lock, Num Lock read/write via HIDIOCSFEATURE)
- Full radio analytics suite

## [2.1.0] - 2025-03-31

### Added
- RSSI helper (C binary, CAP_NET_ADMIN, BlueZ MGMT API opcode 0x0031)
- Battery voltage interpolation from ADC calibration curve
- Battery analytics: type detection (alkaline, NiMH, lithium), discharge rate, remaining time
- Sparkline graphs (voltage, battery, RSSI)
- Prometheus metrics endpoint

## [2.0.0] - 2025-03-31

### Added
- BlueZ Battery Provider (async D-Bus via dbus-fast, Battery1 interface)
- Full HID Feature Report decoding (21 registers)
- Async daemon architecture

### Changed
- Complete rewrite from polling script to async D-Bus daemon

## [1.0.0] - 2025-03-31

### Added
- Initial release: Apple Wireless Keyboard telemetry monitor
- Battery percentage reading via HID Feature Report 0x47
- Waybar/polybar JSON output
- Basic CLI with `--once` and `--status` modes
- PKGBUILD for Arch Linux
- udev rules for hidraw permissions
