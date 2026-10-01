# Features

Every item below was checked against the source (module in parentheses). Line counts: see [ARCHITECTURE.md](ARCHITECTURE.md). This repository covers the Apple Bluetooth keyboard only; the display/DDC/MQTT features were removed (tag `archive/avec-ecran`, issue #59).

## apihub-app (Rust, egui) -- primary interface

Single process, tray-first, 2 tabs (`main.rs`): Keyboard and Diag.

| Area | Feature | Module |
|---|---|---|
| Keyboard | Battery percentage from the kernel `power_supply` node `hid-<mac>-battery*` (the one UPower reads), source of truth; status (charging, discharging, full) | `power.rs` |
| Keyboard | HID Feature Reports via `HIDIOCGFEATURE` (precise battery, raw ADC voltage, calibration curve, firmware, build, name, identity, BT parameters), BCM2042 family only, diagnostics | `keyboard.rs` |
| Keyboard | 17 Apple models identified by PID (USB vendor `05AC` and Bluetooth vendor `004C`) | `keyboard.rs` (`APPLE_MODELS`) |
| Keyboard | Wake/connection events (HID Input Report 0x13), CapsLock/NumLock badges from sysfs | `keyboard.rs` |
| Keyboard | RSSI + TX power through the `rssi-helper` child process (file capability `cap_net_admin`), 1.5 s timeout, 10 s cache | `rssi.rs`, `rssi-helper.c` |
| Keyboard | Battery history (JSONL), 24 h graph, discharge rate and time remaining | `history.rs`, `main.rs` |
| Desktop | BlueZ Battery Provider: one `BatteryProvider1` object per keyboard, so BlueZ creates `org.bluez.Battery1`; re-registered when bluetoothd restarts | `bluez.rs` |
| Desktop | Tray icon (StatusNotifierItem + dbusmenu, pure zbus), re-registered when the StatusNotifierWatcher reappears | `tray.rs` |
| Diagnostics | Diag tab: binaries, user service, `hidraw readable`, keyd config, udev rules, `hid_apple fnmode`, RSSI helper | `main.rs` |

UPower hides the BlueZ battery of keyboards whose kernel driver already publishes a `power_supply` (same MAC serial), so for those the kernel value is what KDE shows; the provider stays a fallback and feeds clients that read `org.bluez.Battery1` directly (see the header of `bluez.rs`).

## Python CLI `apple-kb-monitor` (legacy)

Modes: `--once`, `--status`, `--dump`, `--watch`, `--json`, `--waybar`, `--history`, `--graph`, `--export-csv`, `--metrics` (Prometheus), `--led NAME STATE`. Daemon options: `--threshold`, `--interval`, `--watch-interval`, `--no-provider`. See `apple-kb-monitor --help` for the complete list.

Also in the repository: `apihub-settings` (PySide6 keyboard-only GUI, legacy, not installed by the PKGBUILD) and `rssi-helper.c` (installed, see above).

## System integration files

| File | Purpose |
|---|---|
| `udev/70-apple-kb-hidraw.rules` | `uaccess` tag on Apple hidraw devices (BT `0005:05AC`, `0005:004C`, wired `0003:05AC`); no `input` group |
| `keyd/apple-keyboard.conf` | Apple special keys (device 05ac:0256): F3 Overview, F4 Grid View, F5 Lock, F6 Show Desktop |
| `modprobe/hid_apple.conf` | `fnmode=1` |
| `dbus/com.agenceapi.AppleKbMonitor.conf` | Lets an unprivileged session call `org.bluez` (no bus name is owned) |
| `systemd/apple-kb-monitor.service` | CLI daemon (user) |
| `plasma/com.agenceapi.devicehub/` | Plasma widget (keyboard only, reads `apple-kb-monitor --json`) |
| `kde/DeviceItem.qml` | Bluedevil panel patch |
| `.gitea/workflows/ci.yml` | CI, see [TESTING.md](TESTING.md) |

## Known limits

Tracked in Gitea issues: https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues
