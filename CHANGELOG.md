# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Issues: https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues. One version source: `[workspace.package] version` in `apihub-app/Cargo.toml` = `pkgver` in `PKGBUILD` / `.SRCINFO` = the first entry below (checked by CI, #10).

## [3.1.0] - 2026-10-01

Package `apple-kb-monitor` 3.1.0-15 (`main` @ `8cb044e`). This single entry replaces the two former "[3.1.0]" entries: the 2026-04-02 pre-release described a display / monitor scope that was removed with #59 and now lives in the private repository `lg-ddc-control` (this repository keeps the tag `archive/avec-ecran`); the "Unreleased" one accumulated the work below. Package revisions: -2 scope reduction, -4 daemon + `akmctl` + polkit helper, -6 single language, -7 group `akm`, -8 window fixes, -9 self-check, -10 keyd optional + Apple parity + key mapping, -11 KDE module + notifications + sleep/wake + forget + name + Apple model + French, -12 units enabled by the package, -13 config sections of other programs, -14 user unit without `IPAddressDeny`, -15 doctor journal window.

### Scope and architecture (#8, #59-#62, #66, #16)
- Keyboard-only repository: the display, brightness, MQTT and Home Assistant code was removed (#59, #74); `apihub-app` keeps the keyboard tabs.
- Cargo workspace with a hardware-free core `akm-core`, a headless daemon `apple-kb-monitord` (single owner of the keyboard, `systemd --user`, D-Bus `com.agenceapi.AppleKbMonitor1`, BlueZ provider, tray), thin clients over D-Bus (`akmctl`, `apihub-app`, Plasma widget), event-driven actor on BlueZ signals (#60, #61, #62, #66).
- Rust is the only language of the driver: the interpreted CLI `apple-kb-monitor`, `apihub-settings`, their unit and tests were removed; `akmctl` covers `history` (+ `--since/--until/--last`, `export --csv`, `import`), `graph`, `waybar`, `metrics`, `led`, a safe `dump` (`0x47`/`0x46`/`0x49` only); one history file `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl` (#16, #17, #22). The package no longer depends on an interpreter (3.1.0-6).
- Daemon D-Bus API v2: per-device objects `/devices/<MAC>`, `.Input`, `.Link`, `.Keymap`, `.Tray` interfaces, `StateChanged` and per-device signals, `Json` property (#92, #93).

### Battery (#65, #178-#180, #146, #213, #215, #189)
- The kernel `power_supply` node (what UPower reads) is the source of the displayed percentage, shown as "keyboard indication" and never rewritten; raw HID reports are telemetry (#65).
- Real voltages `0x46` / `0x49` in mV in the history (schema 2); the former constant pseudo-voltage is marked `voltage_valid: false` and ignored (#180).
- Charge **estimate** by battery chemistry (`[battery] chemistry`: alkaline / NiMH / lithium / unknown) with a range, grace period after a battery change, time remaining; alerts computed on the estimate when there is one (#178). Battery-change detection and per-set log (#85). Forecast in days (#83).
- The keyboard's percentage only steps down at reconnections: the age of the last reading is shown everywhere and a drop seen on the first reading after a reconnection raises no alert (#179).
- "Apple display" percentage (IOBluetooth curve, `[display] apple_percent`) (#213); firmware thresholds Full / Low / Critical / Empty (`0x60`) read once per connection, margins shown (#215).
- Keyboard-driven alerts from Input `0x30` ("battery low / critical (keyboard alert)"), `BatteryAlert` signal, CapsLock flash on critical, deduplicated with the percentage alerts (#189).
- Multi-threshold alerts with hysteresis (default 30 / 15 / 5 %), connection notifications, PowerDevil dedupe (#82, #84, #254).
- Read cost: the daemon follows the safe read policy; UPower's 30 s polling is documented and optionally removed (`bluetooth/akm-conf.py upower`) (#146).

### Apple parity and register map (#227, #189-#191, #244, #251, #217, #248, #192)
- Declarative register map `akm-core/src/registry.rs` of every known HID report (57 entries) with safety classes; read allow-lists generated from it; the hidraw doors ask it first; tests sweep the 256 ids in the three directions (#227).
- Firmware version `0x4F` read once per connection and compared with an embedded, dated table; `akmctl firmware`, `akmctl info` (never reads the keyboard); D-Bus `Firmware*` properties (#227).
- Pure model of Apple's macOS 26.5 driver (`apple_model.rs`, rules R1-R8 with sources): battery schedule 60 s then 4 h (1 h after a failure), GET Feature `0x47` then **GET Input `0x30`** then `0x46` / `0x49` 1 s apart, 3.5 s timeout, circuit breaker after 3 silences (2 after a sleep), a HANDSHAKE refusal resets the counter, BlueZ `Device1.Disconnect` after the third silence (`[apple] disconnect_on_breaker`), never `RemoveDevice` (#251; the full breaker asked by #243).
- The breaker reaches every emitter: published in `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state`, read by `akm-hid-control` (root) and by the `akmctl` write door; CapsLock flash blocked while open (#244, #251).
- `WillShutdown` (Feature `0x40`, id alone) sent once at shutdown / restart as macOS does, default on (`[apple] will_shutdown`): logind `PrepareForShutdown` inhibitor, `akmctl shutdown-notify`, user unit `apple-kb-monitor-shutdown.service` (#191).
- Keyboard off (Input `0x13` bit 1 = 0) distinct from a link loss: `PoweredOff`, `KeyboardOff` signal, "switched off" notification (#190).
- HID_CONTROL SUSPEND (`0x13`) before sleep and EXIT_SUSPEND (`0x14`) at wake, one byte on `bluetoothd`'s control socket (`pidfd_getfd`), root system units `apple-kb-monitor-suspend/resume.service`, `/etc/apple-kb-monitor/hid-suspend.conf`, `akmctl hid-control … --dry-run`, polkit action `hid-control` (#244).
- Clean forget like macOS: `akmctl repair` on a connected keyboard sends SET Feature `0x41` `RecantConnection` after pre-flight, host-side backup and a typed `OUBLIER`, waits 2000 ms, then `RemoveDevice`; nothing removed if `0x41` fails (#217).
- **Name stored in the keyboard, an ordinary command (#248).** Measured on the real keyboard on 2026-10-02 (firmware `0x0050`): the SET Feature `0x55` is accepted and `0x51`-`0x54` show the new name in the same connection (no power cycle). `akmctl rename --device-name <name>` now **writes**: pre-flight, backup 0600, ONE confirmation `[y/N]` / `[o/N]` (`--yes` skips it and works without a terminal), one write, immediate read-back through the hidraw door, verdict; exit codes 10 no confirmation possible, 11 pre-flight, 12 cancelled, 13 written but not read back, 14 read back different (rollback command printed). Short output, `--verbose` for the bytes. Removed: the typed word, the name typed again, the configuration lock (`[apple] allow_device_name_write` is obsolete: read, ignored), the power cycle and its 180 s wait. `--write-device-name` is accepted and does nothing. New D-Bus method `RereadName()` (the daemon forgets `0x51`-`0x54` and reads them again, at most once per 30 s). Settings module: the "Name" tab asks one confirmation and runs `akmctl … --yes` itself (QProcess, no shell, 90 s), result shown in the page; no terminal any more. Unchanged: register map, `WriteSession`, read budgets, breaker, MTU ≥ 66, backup before the write, never a retry. Persistence across a battery change is still not measured.
- Name stored in the keyboard, first version: `akmctl rename --device-name <name> [--dry-run] [--write-device-name] [--show] [--restore]`; the Lion `setDeviceName:` frame (SET Feature `0x55`, 65 bytes) established by disassembly; the real write behind **three locks** (`[apple] allow_device_name_write = true`, control-channel MTU ≥ 66 read by `akm-hid-control inspect`, name typed again); D-Bus `DeviceNameOnKeyboard`, `akmctl status` line `On kb:` (#192, #248).
- Writes exist only as named operations (`Shutdown` `0x40`, `Forget` `0x41`, `DeviceName` `0x55`), each once per session with its exact length; one write function with two fixed sizes, every byte logged, never retried; a source scan fails on a second write path. Not implemented on purpose: `0x44` `FullFactoryDefault`, `0x4A` SCO notification (#216).

### Link: reconnection, doctor, repair (#142, #143, #146, #206, #214)
- Root cause of the "keyboard does not reconnect" episodes measured and documented (radio silence under request bursts, BlueZ paging given up, adapter USB autosuspend); recovery state machine (connected, dormant, unreachable, auth-failed, suspended), paging cadence, logind sleep inhibitor (#142).
- `akmctl doctor [--json]`: adapter, pairing, link, link key (sudo), hidraw, adapter power management, BlueZ `FastConnectable` / `Reconnect*`, UPower `NoPollBatteries`, `bluetoothd` journal over the last 6 h plus the boot (3.1.0-15), daemon; every finding with its fix. `akmctl repair [--force]`: wake + reconnect first, re-pairing only after a typed confirmation.
- `bluetooth/akm-conf.py {bluez,upower} [--apply]` and `udev/61-akm-bt-adapter-no-autosuspend.rules` (not installed by the package) (#143).
- `0xFE` is never requested (two link losses), `0x4C` never requested; a read requires a key press in the last 60 s outside the daemon (link losses of #175).
- `Refresh()` bounded to one per 5 min; 1 s between requests; circuit breaker (#206, #214).

### Security (#63, #69, #73, #155, #202-#212, #214, #209)
- udev rule `70-apple-kb-hidraw.rules` tags the hidraw node `uaccess` (logind ACL for the active seat), no `input` group; narrowed to the 17 Bluetooth product ids of the model table, wired Apple keyboards no longer matched; the keylogger trade-off is documented (#63, #73, #155, #204).
- `rssi-helper`: the only binary with `cap_net_admin`, applied by `post_install`; `root:akm 0750`, group `akm` created by sysusers (#69, #209).
- `SetFnMode` runs `/usr/bin/pkexec akm-helper set-fnmode <n>` from constants, caller uid checked, one dialog at a time; polkit `auth_admin` without `_keep`; `SetSwapOptCmd` / `SetIsoLayout` D-Bus methods removed (#202, #203). Plain-text keyboard names in the widget and escaped tray tooltip; aliases cannot start with `-` (#205, #207). Private per-uid `hid.lock` directory, `O_NOFOLLOW` (#208). Systemd unit options compatible with pkexec and file capabilities (#211); `IPAddressDeny` removed from the user unit (3.1.0-14). Dependencies updated, `deny.toml` documents the remaining advisories (#212; the eframe 0.29 advisories stay tracked in #224). Real `0x4C` fingerprints removed from fixtures (#210).
- Sensitive bytes of `0x4C` never published (#200, #201).

### Keys (#246, #247, #125, #88)
- keyd became optional: `keyd reload` crashes keyd 2.6.0 (reproduced, patch and bug report prepared), the package never reloads nor restarts it; the config is an example under `/usr/share/doc` (#246).
- Special keys modelled from the kernel sources (udev hwdb → `hid_apple` → xkb / KDE): `akmctl keys [--check] [--all]`, Keys tab in the window, manual mapping without keyd (`keymap.toml` profiles and presets → validated udev hwdb installed by `pkexec akm-keymap-helper`, polkit action `install-keymap`), `akmctl keymap kde-apply` for F4 / Eject (#247).
- `akmctl get/set fnmode|param` and `--persist` through `akm-helper`; Fn lock toggle from the tray (#88). LEDs through evdev, via keyd's virtual keyboard when keyd grabs the device (#125).

### KDE Plasma 6 (#249, #250, #252-#254, #114, #115-#118, #121, #122, #86, #87, #21)
- Tray in the daemon: StatusNotifierItem + dbusmenu, symbolic icons by level / charging / disconnected, rich tooltip and menu, re-registration when the watcher reappears (#115, #116, #86, #87). `apihub-app` single instance through D-Bus activation `com.agenceapi.AppleKbMonitor`, reverse-DNS desktop file, no autostart (#115, #121, #21).
- Plasma widget `com.agenceapi.devicehub` driven by D-Bus signals, claims the notification area so there is one icon only; French catalogue (#117, #253, #114, #122).
- Window integrated with the desktop: Wayland `app_id`, theme / accent / contrast, French (156 strings); vsync, bounded D-Bus calls, history off the UI thread, heartbeat file (#118, #230-#236).
- KNotification events (`apple-kb-monitor.notifyrc`, 14 events, buttons Open / Repair… / Ignore, replacement per slot, FR/EN) (#249). PowerDevil dedupe: one `BatteryEstimate` reminder when KDE already warns (#254). Keyboard forgotten from Plasma removed at once, one `KeyboardRemoved` notification (#252). `akmctl status` explains the two names (BlueZ alias vs kernel name) (#248).
- System Settings module "Apple Keyboard" (`kcm_applekeyboard`, C++ plugin + QML pages State, Keys, Notifications, Name, Diagnostics), no I/O on the GUI thread, writes `config.toml` atomically, uses the existing polkit actions and D-Bus methods only (#250).
- Alias of the keyboard on this computer: `akmctl rename`, D-Bus `SetAlias`, tray, window, widget (#141).

### Quality (#241, #9, #24, #10)
- Local pipeline `scripts/ci-local.sh` (18 steps: versions, secrets, fmt, clippy, test, claims, redaction, deny, audit, udev, qml, shell, c, security, units, plasma, package, e2e), end-to-end tests of the window and daemon under Xvfb + bubblewrap with a simulated keyboard (11 scenarios + KCM), `akmctl selftest` every 15 min with notifications and optional Gitea issues, `.githooks/pre-push`, Gitea Actions CI with five jobs (#241, #9, #24).
- Registry-backed claim check (`qa_checks.py claims`): documents and strings may not state the claims refuted by the counter-audit (tracks #228). Redaction check: no Apple binary or decompiled code committed.
- Fixtures of the real A1314 ISO, synthetic reports, Lion frames, other models; `tests/live/check_keyboard.sh` read-only hardware harness; tooled audits (miri, udeps, fuzz, proptest) and adversarial reviews in `docs/`.

### Packaging (#2, #3, #5, #6, #7, #14, #156)
- PKGBUILD with local sources, `!lto`, `backup=` for `hid_apple.conf` and `hid-suspend.conf`, dependencies declared for the dlopen'ed Wayland / X11 / GL libraries and the KDE module (#2, #7, #14, #156); `.SRCINFO` regenerated (#3); desktop categories and D-Bus files fixed (#6); widget launches the window (#5).
- The package ships its unit links (`default.target.wants`, `suspend.target.wants`) and enables nothing with `systemctl`; a `pre_remove` helper disables only links it created under `/etc/systemd`. It re-applies the group and the capability of `rssi-helper` on every install / upgrade. Completions and manual generated by `akmctl` at build time.

### Fixed
- keyboard: HID fd leak, wake-monitor busy loop after disconnect, unvalidated calibration, PID substring match, NUL in device name, CapsLock LED (#52, #53, #79); `power_supply` path without `-NN` suffix, null percentage without sysfs fallback (#70, #71); RSSI reply matched to its request, 127 rejected, strict MAC (#56, #31, #77); BlueZ `PropertiesChanged`, re-registration, log flood (#57, #72, #81); alerts re-armed, no 100 % / 0 V points, tray SNI retries, UTF-8 safe slicing (#28-#32, #35, #36, #38, #74); invalid history points rejected (#39); no 0 % exported to BlueZ on a failed read (#78).
- Daemon: no warning for the config sections of another program sharing `config.toml` (3.1.0-13).

### Documentation
- README, INSTALL, CONFIGURATION, FEATURES, TROUBLESHOOTING, ARCHITECTURE, TESTING rewritten for 3.1.0-15, `docs/INDEX.md` added, Gitea wiki updated (#13, #255). Reverse-engineering and audit documents (`docs/RE-*.md`, `HARDWARE-*.md`, `AUDIT-*.md`, `CONTRE-AUDIT.md`, `VERIF-BATTERIE.md`) are dated evidence and are not rewritten.

### Known open items
- Hardware validation of 16 of the 17 models (#23); multi-keyboard UI (#119, #94); Magic Keyboard battery report `0x90` (#95); the effect on the keyboard of `WillShutdown`, `0x41` and the sleep / wake bytes is not observable; the name write (`0x55`) was never exercised on hardware (#248). Open milestones: v3.2 backlog, v4.0 keyboard differentiation.

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
