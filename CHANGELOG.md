# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [3.1.0] - Unreleased

Audit and debug pass (tracking issue #8), then scope reduction: the repository is now keyboard-only. Bugs are tracked one per Gitea issue (label `bug`); "Fixes #N" is in the commit messages of `617c3db..HEAD`. Single version source: `[workspace.package] version` in `apihub-app/Cargo.toml` = `pkgver` in `PKGBUILD`/`.SRCINFO` = first entry of this file (checked by CI, #10).

### Removed (breaking)
- **Display / DDC / MQTT / Home Assistant**: `ddc.rs`, `brightness.rs`, `mqtt.rs`, `ddc-tool`, `mqtt-bridge.py`, DDC tests, display docs, `config.toml.example`, KDE brightness shortcuts, the `i2c` group step, the `python-paho-mqtt` optional dependency (#59, #74). `apihub-app` now has two tabs: Keyboard and Diag. The code lives in the private repository `lg-ddc-control` (this repository keeps the tag `archive/avec-ecran`).
- The previous `[3.1.0] - 2026-04-02` entry below describes that display feature set, which no longer exists in this repository.

### Changed
- **Permissions**: udev rule `70-apple-kb-hidraw.rules` tags Apple hidraw devices `uaccess` (logind ACL for the active seat user) instead of the `input` group. Matches Bluetooth vendors `05AC` and `004C` and wired Apple keyboards (`0003:05AC`). No `usermod -aG input` any more (#63, #73).
- **Battery source**: the kernel `power_supply` node (`hid-<mac>-battery`, the one UPower reads) is the source of truth for the percentage; raw HID reports are diagnostics (#65, `power.rs`).
- **RSSI**: read through a dedicated C helper `rssi-helper` installed in `/usr/lib/apple-kb-monitor/` with `cap_net_admin+ep` applied by `post_install` (`setcap`); the GUI stays unprivileged and runs it as a child process with a timeout and a 10 s cache (#69).
- **BlueZ battery provider**: one `BatteryProvider1` object per keyboard under `/com/agenceapi/AppleKbMonitor`, zbus `ObjectManager`, adapter path resolved from BlueZ, automatic re-registration when bluetoothd restarts, no well-known D-Bus name requested (#67, #72, #81).
- **Keyboard model table** rebuilt from `hid-ids.h` (17 models); vendors `05AC` and `004C` accepted; raw HID telemetry reserved for the BCM2042 family (#64).
- Packaging: PKGBUILD uses local sources, `!lto`, `backup`, correct depends/optdepends (#2, #7); `.SRCINFO` regenerated (#3); desktop `Categories` and udev URL fixed, D-Bus policy restricted (#6); Plasma widget launches `apihub-app` (#5).

### Fixed
- **keyboard**: leaking HID fd, wake-monitor busy loop at 100 % CPU after disconnect, unvalidated calibration, PID match by substring, NUL in device name, CapsLock LED (#52, #53); wake-monitor started on demand even when the keyboard is absent at launch, timestamp exposed (#79).
- **power / sysfs**: `ps_path` without `-NN` suffix gave an empty sysfs block, `battery.percentage` was null without sysfs fallback when hidraw is unreadable (#70, #71).
- **rssi**: MGMT reply matched to its request, 127 rejected, strict MAC (#56); poll keeps the last RSSI between HID re-reads (#31); Python MGMT fallback aligned (#77).
- **bluez**: `PropertiesChanged` on `Percentage`, re-registration, validated MAC (#57); provider alive again after the D-Bus policy change (#72); no log flood on registration failure (#81).
- **poll / tray / UI**: battery alerts re-armed, no 100 % / 0 V points, Remaining cleared (#31, #32); tray retries SNI registration and re-registers when the StatusNotifierWatcher reappears, scroll saturates (#38, #74); Quit closes the window, diag guard, UTF-8 safe slicing, tooltip `n/a` (#28 to #30, #35, #36); polling thread supervised (restart after panic).
- **history**: invalid points (voltage <= 0, NaN) rejected on write and ignored in the estimate (#39).
- **Python CLI / settings**: hardened HID/sysfs access, SDP parsing, history, daemon loop and `--json` serialisation (#40 to #51); no 0 % exported to BlueZ when the HID read fails (#78); calibration validated like the Rust side (#80).

### Added
- Rename the keyboard on this computer (BlueZ alias): `akmctl rename <name>|--reset`, D-Bus `SetAlias` + `Name` property, tray "Rename keyboard…", name field in the window and the Plasma widget, name in tooltips and JSON; research on the in-keyboard name in docs/RENOMMER-CLAVIER.md (#141).
- `tests/live/check_keyboard.sh`: read-only checks on real hardware (sysfs vs UPower vs BlueZ vs CLI battery, hidraw rights, udev `uaccess`, keyd, services).
- Fixtures for a real A1314 ISO under `tests/fixtures/` and CI in `.gitea/workflows/ci.yml` (cargo build/clippy/test, pytest, shellcheck, gcc on `rssi-helper.c`, `udevadm verify`) (#9, #24).
- Adversarial review documents: `docs/REVUE-ARCHITECTURE-GLOBALE.md`, `docs/REVUE-ARCHITECTURE-CLAVIER.md`, `docs/REVUE-CORRECTIFS.md` (issues #72 to #81).

### Documentation
- README, ARCHITECTURE, FEATURES, CONFIGURATION, INSTALL, TROUBLESHOOTING, TESTING rewritten for the keyboard-only scope; Gitea wiki updated.

### Known open items
- Not fixed by a commit in this range: #75 (RSSI carried over without age or MAC check), #76 (valid RSSI discarded when only the TX power is 127). Some of the fixes above (#25 to #27, #33, #34, #37, #54, #55, #58) concerned display code that was then removed with #59.
- The Python CLI `apple-kb-monitor` and `apihub-settings` still contain MQTT / `ddc-tool` code paths from before the split; they are outside this documentation pass.
- Architecture roadmap (workspace + testable core, headless daemon, thin clients): #60 to #62, #66, #68.

## [3.1.0] - 2026-04-02

Broad feature expansion: 10 keyboard models, system tray, battery analytics, App Presets, expanded MQTT entities, and DDC/CI improvements.

### Added

#### Keyboard
- **10 Apple keyboard models** supported (was 3) -- A1016, A1255 ANSI/JIS, A1314 ISO/ANSI/JIS, A1644 ANSI/ISO, A2449 ANSI/ISO
- **Wake event monitor** -- dedicated thread monitors HID Input Report 0x13 (vendor FF01 wake/connection events)
- **LED state display** -- CapsLock and NumLock badges read from sysfs, shown in Keyboard tab
- **Battery time remaining estimate** -- discharge rate calculation from history with time-to-empty prediction
- **Battery history graph** -- painter-based 24h chart (battery % + voltage dual axis) rendered in the Keyboard tab

#### Display / DDC
- **Video Black Level RGB** -- 3 new writable VCPs discovered (0x6C, 0x6E, 0x70) with slider controls
- **VCP 0x02 smart polling** -- reads New Control Value flag first; skips WARM tier VCPs when nothing changed on the monitor OSD
- **Factory Reset buttons** -- VCP 0x04 (restore factory defaults), 0x05 (restore factory brightness/contrast), 0x08 (restore factory color) with confirmation dialog
- **PBP mirror registers display** -- reads sub-display brightness, contrast, and color preset (0xE8, 0xE9, 0xEA) when split mode is active
- **All Advanced VCPs brute-force verified** -- 7 corrections from hardware verification against LG 34GN850

#### Desktop integration
- **System tray** -- ksni-based KDE StatusNotifierItem (D-Bus protocol), scarab icon, battery tooltip, quit menu
- **App Presets** -- automatic picture mode switching based on active window class (e.g., Firefox -> sRGB, Steam -> FPS 1)
- **KWin D-Bus scripting** -- Wayland-native window class detection via `org.kde.kwin.Scripting` (replaces xdotool)

#### MQTT / Home Assistant
- **RSSI sensor** -- keyboard signal strength in dBm
- **TX power sensor** -- keyboard transmit power in dBm
- **Connected binary_sensor** -- keyboard connectivity state (ON/OFF)
- **Volume number entity** -- bidirectional monitor volume control (0-100%)
- **Picture mode select** -- 14 modes (Custom, Reader, Vivid, HDR Effect, Cinema, Color Weakness, FPS 1/2, RTS, sRGB, DCI-P3, EBU, Photo, Calibration), bidirectional
- **Input source select** -- DisplayPort, HDMI 1, HDMI 2, bidirectional
- Total: **15 HA entities** (was 4)

### Changed
- **Dependencies** -- removed `gtk`, `tray-icon`, `png` crates; added `ksni` for D-Bus native system tray
- **LOC** -- apihub-app grew from 3030 to ~3900 LOC (main.rs 1477->2126, keyboard.rs 330->401, ddc.rs 340->355, mqtt.rs 240->339, history.rs 66->90)
- **Polled VCPs** -- 34 VCPs across 4 tiers (HOT/WARM/COLD/PBP), up from flat polling
- **Writable VCPs** -- 22 writable controls (was ~15)

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
