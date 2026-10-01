# Features

Every item below was checked against the source (module in parentheses). Line counts: see [ARCHITECTURE.md](ARCHITECTURE.md). This repository covers the Apple Bluetooth keyboard only; the display/DDC/MQTT features were removed (tag `archive/avec-ecran`, issue #59).

## Daemon `apple-kb-monitord` and window `apihub-app` (Rust)

`apple-kb-monitord` is the single owner of the keyboard (D-Bus `com.agenceapi.AppleKbMonitor1`, tray, notifications); `apihub-app` is the egui window (2 tabs, `apihub-app/src/main.rs`: Keyboard and Diag). Shared logic lives in `akm-core`.

| Area | Feature | Module |
|---|---|---|
| Keyboard | Battery percentage from the kernel `power_supply` node `hid-<mac>-battery*` (the one UPower reads, = Feature `0x47`), source of truth | `akm-core/src/power.rs` |
| Keyboard | HID Feature Reports via `HIDIOCGFEATURE`, BCM2042 family only: daemon allow-list `0x47`, `0x46` (voltage mV LE, checked against `0xFF` BE), `0x49` (smoothed voltage), only after a key press in the last minute, under a cross-process lock; never `0xFE` | `akm-core/src/read_policy.rs`, `decode.rs` |
| Keyboard | Decoded values: voltage (mV), discharge table `0x5A` (mV), firmware version `0x0050`, device name `0x51-0x54`, paired host address (`0x4C` bytes 2-7 only; the other 12 bytes are never published). Unknown reports stay raw | `akm-core/src/decode.rs` |
| Keyboard | 17 Apple models identified by PID (USB vendor `05AC` and Bluetooth vendor `004C`) | `akm-core/src/model.rs` (`APPLE_MODELS`) |
| Keyboard | Passive input events (Input `0x13`, `0x04`/`0x05`/`0x30` listened to, never requested); CapsLock/NumLock badges from sysfs | `akm-core/src/passive.rs`, `apple-kb-monitord/src/passive.rs` |
| Keyboard | RSSI + TX power through the `rssi-helper` child process (file capability `cap_net_admin`), 1.5 s timeout, 10 s cache. BR/EDR RSSI is a relative gap in dB, not dBm (#174) | `akm-core/src/rssi.rs`, `signal.rs`, `rssi-helper.c` |
| Keyboard | Battery history (JSONL), discharge rate, estimates by chemistry (estimate, not a measurement) | `akm-core/src/history.rs`, `forecast.rs`, `chemistry.rs` |
| Link | Reconnection state machine (connected, dormant, unreachable, auth-failed, suspended), logind sleep inhibitor | `akm-core/src/recovery.rs`, `apple-kb-monitord/src/sleep.rs` |
| Desktop | BlueZ Battery Provider: one `BatteryProvider1` object per keyboard, so BlueZ creates `org.bluez.Battery1` | `apple-kb-monitord/src/bluez.rs` |
| Desktop | Tray icon (StatusNotifierItem + dbusmenu, zbus) | `apple-kb-monitord/src/tray.rs` |
| Diagnostics | Diag tab: binaries, services, `hidraw readable`, keyd config, udev rules, `hid_apple fnmode`, RSSI helper; `akmctl doctor` / `repair` | `apihub-app/src/main.rs`, `fnmode_diag.rs`, `crates/akmctl` |

## Python CLI `apple-kb-monitor` (legacy)

Legacy, kept for scripts. Known defects: refuted decodings and `0x4C` bytes published (#200), no safe read policy and `--dump` reads `0xFE` (#201). Modes: `--once`, `--status`, `--dump`, `--watch`, `--json`, `--waybar`, `--history`, `--graph`, `--export-csv`, `--metrics` (Prometheus), `--led NAME STATE`. Daemon options: `--threshold`, `--interval`, `--watch-interval`, `--no-provider`. See `apple-kb-monitor --help` for the complete list.

Also in the repository: `apihub-settings` (PySide6 keyboard-only GUI, legacy, not installed by the PKGBUILD) and `rssi-helper.c` (installed, see above).

## System integration files

| File | Purpose |
|---|---|
| `udev/70-apple-kb-hidraw.rules` | `uaccess` tag on Apple hidraw devices (BT `0005:05AC`, `0005:004C`, wired `0003:05AC`); no `input` group |
| `keyd/apple-keyboard.conf` | Apple special keys (device 05ac:0256): F3 Overview, F4 Grid View, F5 Lock, F6 Show Desktop |
| `modprobe/hid_apple.conf` | `fnmode=1` |
| `dbus/com.agenceapi.AppleKbMonitor1.service`, `dbus/com.agenceapi.AppleKbMonitor.service` | D-Bus activation of the daemon and of the window |
| `systemd/apple-kb-monitord.service` | Rust daemon (user) |
| `systemd/apple-kb-monitor.service` | legacy Python daemon (user), conflicts with the Rust daemon |
| `plasma/com.agenceapi.devicehub/` | Plasma widget (keyboard only, D-Bus `com.agenceapi.AppleKbMonitor1`) |
| `.gitea/workflows/ci.yml` | CI, see [TESTING.md](TESTING.md) |

## Known limits

Tracked in Gitea issues: https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues
