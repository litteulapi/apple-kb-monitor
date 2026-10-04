# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Issues: https://github.com/litteulapi/apple-kb-monitor/issues. One version source: `[workspace.package] version` in `apihub-app/Cargo.toml` = `pkgver` in `PKGBUILD` / `.SRCINFO` = the first entry below (checked by CI).

## [3.3.0] - 2026-10-05

Changes since 3.2.0-2 (cross-audit rounds 3 to 8).

### Changed
- `akmctl`, the daemon and `akm-helper` are written in English and translated through gettext; the command line, the System Settings module, the widget, the notifications and the polkit messages come in 19 languages.
- Battery history per keyboard: every `history.jsonl` line carries the `mac` of the keyboard that gave it. At the first start of this version the lines of earlier versions are given once to the first keyboard followed (copy kept as `history.jsonl.pre-mac`); battery sets, forecast and advice use only that keyboard's lines.
- `apple-kb-monitord --batteries` groups the sets by keyboard, and `--batteries --json` now prints `[{"mac", "sets": [BatterySet]}]` instead of `[BatterySet]`.
- Name stored in the keyboard, an ordinary command: `akmctl rename --device-name <name>` writes after a pre-flight, a backup and one confirmation (`--yes` skips it), then reads the name back. The Name tab of the settings module runs it through the daemon, without a terminal. New D-Bus method `RereadName()`.
- `akmctl repair` and `akmctl rename` (also `--show` and `--restore`) act on the keyboard given by `--mac` and open that keyboard's own hidraw node, never the first Apple keyboard found; `doctor` and `repair` use that keyboard's adapter instead of `hci0`.
- `akmctl doctor` (and `selftest`, and the pre-flight of `repair` and `rename`) no longer starts a stopped daemon through D-Bus activation; neither does the settings module.
- User units come with a systemd preset: the package creates no link and starts nothing. Enable them once with `systemctl --user enable --now apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer`; an upgrade from an older version prints this command.
- Settings module: keyboard names, aliases and daemon messages are shown as plain text, never as markup; it names the configuration file it edits.
- Widget: the two Fn-mode menu entries are radio items; dates, times and percentages follow the user's locale.
- Fn mode 4 (`fkeysdisabled`) is accepted wherever a Fn mode is read.

### Fixed
- Battery story, forecast and replacement detection no longer mix keyboards when several are paired or none is followed.
- `akmctl doctor --mac` shows the link health and stored link key of that keyboard and its adapter; the journal finding says what it covers: "no bluetoothd error in the last 6 hours of this boot".
- Settings saved from the module: a write over a file changed in between is refused, a symlinked `config.toml` is kept, D-Bus deadlines above 25 s are honoured.
- Right-click menu no longer emptied by a state update while open; the widget keeps no error, check or Fn mode of a stopped daemon.
- Daemon: blocking work moved off the D-Bus executor, no call left unanswered after a panic, no start-up race on the bus name, no BlueZ polling, link statistics and breaker age right after a suspend.
- Keymap: the hand-over lock covers Apply, Reset and the `akmctl keymap` rollback; Undo on the Keys page keeps the `hid_apple` controls working.
- `akmctl history import` no longer loses samples written by the daemon at the same time.
- Messages left in English or glued from fragments in `akmctl`, the daemon, the settings module and the widget.
- No panic on a non-UTF-8 argument (daemon, `akmctl`, `akm-helper`); `akmctl history` dates accept a UTC offset or `Z` and refuse impossible dates.

### Security
- Notification actions and unicast signals sent by another peer of the session or system bus are refused.
- `Keymap.Apply`, `Keymap.Reset` and `Settings.RunAkmctl` check the caller's uid; `RunAkmctl` runs a closed list of commands.
- The shipped daemon runs `akmctl` by its absolute path, without environment override; the privileged helper vets its locale and reads its configuration without reopening the path by name; the symlink-following writer is no longer compiled into the root helper.
- `breaker.state` is written 0600; full Bluetooth addresses no longer appear on the widget DATA tab nor in the copied diagnosis.

### Tests
- Every D-Bus test runs on a private bus with a private `XDG_RUNTIME_DIR`: no test can reach or activate the installed daemon, and none leaves sockets or fake helpers in `TMPDIR`.
- The local CI also builds the settings module and runs the widget tests, the catalog checks (three generators, stable output), shellcheck, Qt 6 qmllint and cargo deny; a skipped required step fails it, and the public tree is checked for private tracker references and internal ids.
- End-to-end tests of the settings module and the widget wait on conditions instead of fixed delays.

## [3.2.0] - 2026-10-04

### Simpler
- One interface: the Plasma widget of the notification area (left click: panel, right click: full menu). The egui window `apihub-app`, the daemon's own tray icon and the KRunner plugin are gone.
- The widget is added to the notification area by itself (`EnabledByDefault`).
- System Settings module rewritten in QML; every read and write goes through the daemon's new D-Bus interface `…1.Settings` (allow-listed keys, other sections never read back).
- No group `akm`, no restart: `rssi-helper` answers only for a connected Apple keyboard and ships with its capability; the daemon starts by itself in open sessions.
- One privileged helper (`akm-helper <command>`) instead of five; polkit messages in French.
- Dead code and unused tools removed.

### Fixed
- Right-click menu of the tray icon shows every entry again.
- Reaudit of 3.1.0-28: its findings are fixed.
- 3.2.0-2: links point to GitHub, no private host name or address left in the sources; `akmctl selftest --gitea-issue` reads its server from `AKM_GITEA_API` and `AKM_GITEA_REPO`.


### Daemon
- History file bounded to 5 MiB (oldest ordinary samples dropped down to 4 MiB, battery replacements kept, `.prev` copy); D-Bus `History(since)` thinned by the daemon to 2000 points and new `HistoryMax(since, max)` on the root and keyboard objects.
- D-Bus `Diagnose() -> s`: the checks of the window's Diag tab run by the daemon and returned as JSON (`{schema, daemon_version, passed, total, checks: [{id, label, ok, detail}]}`), in the daemon's language; the hidraw node is tested with `access(2)`, never opened; programs bounded to 3 s.
- Quiet hours `[notifications] quiet_hours = "22:00-07:00"`: inside the range a non-critical notification is journalled and held (one per replacement slot), then shown when the range ends; critical ones always pass; what waits survives a restart (`deferred-notifications.json`).
- Notification button "Remind me tomorrow" on `BatteryLow`, `BatteryEstimate`, `BatteryReminder` and `FirmwareUpdate`: the same notification is shown again 24 hours later, also after a daemon restart; new batteries cancel a pending battery reminder.
- Battery health advice "batteries changed too often" after two consecutive sets of less than 30 days: `battery_advice` in `GetState`, a line in the tray menu and tooltip, one `BatteryAdvice` notification when the second set is replaced (`[notifications] battery_advice`).
- Usage statistics without key logging, off by default (`[usage] active_time`): active minutes per day counted from the mere arrival of input reports (`akm_core::activity::note()` takes no argument), `usage` in `GetState` (7 days), one counter per day kept 90 days in `usage.json`.
- Plasma OSD (`org.kde.osdService`) when the Fn mode or the layout of `hid_apple` changes (`[osd] fn_mode`) and when Caps Lock is pressed (`[osd] caps_lock`); Caps Lock badge on the tray icon (`OverlayIconName`).
- Link quality: every disconnection recorded with its reason, counts per hour and per day, 7 days of relative signal by the hour (`link-quality.json`, bounded); "unstable link" notification above 3 disconnections within an hour, once per episode (`[notifications] link_unstable`); `link_quality` in `GetState`, `quality` in `Link.Status()`, new `Link.Quality()`, a `link-quality` line in `akmctl doctor`.
- Tray entries "Reconnect", "Disconnect" and "Forget this keyboard…": `Device1.Connect` through the recovery machine (20 s spacing), `Device1.Disconnect` (the daemon then pages nothing until asked), and a forget that is only **asked**: a `ForgetConfirm` notification whose single button, pressed within 60 s by the notification server itself, makes the daemon call `Adapter1.RemoveDevice`; `Link.Disconnect(mac)` and `Link.RequestForget(mac)` on D-Bus; nothing is written to the keyboard.
- Several keyboards: `devices` in `GetState` lists every paired Apple keyboard (the one the daemon reads first, the others from BlueZ with the battery BlueZ `Battery1` or UPower already holds: nothing more is asked from any keyboard); one D-Bus object per keyboard; the tray icon, status and title follow the weakest connected keyboard and the menu and tooltip list them all; `Link.Status()` entries carry `connected` and `battery`.
- Settings per keyboard: the Fn mode set through a keyboard's D-Bus object is remembered for that address (`device-settings.json`) and, when that keyboard reconnects with another mode in effect, offered back in a `SettingsReapply` notification (button "Apply", polkit authentication), applied at once with `[devices] reapply_settings = "auto"`, or left alone with `"off"`; the notification says the setting is common to every Apple keyboard.

### Tests
- Every D-Bus test of the daemon runs on a private `dbus-daemon` whose configuration has no service directory (`apple_kb_monitord::testbus`), session and system bus alike: a test can no longer activate the installed daemon nor reach the real BlueZ.

## [3.1.0] - 2026-10-01

Package `apple-kb-monitor` 3.1.0-15 (`main` @ `8cb044e`). This single entry replaces the two former "[3.1.0]" entries: the 2026-04-02 pre-release described a display / monitor scope that was removed and now lives in the private repository `lg-ddc-control` (this repository keeps the tag `archive/avec-ecran`); the "Unreleased" one accumulated the work below. Package revisions: -2 scope reduction, -4 daemon + `akmctl` + polkit helper, -6 single language, -7 group `akm`, -8 window fixes, -9 self-check, -10 keyd optional + Apple parity + key mapping, -11 KDE module + notifications + sleep/wake + forget + name + Apple model + French, -12 units enabled by the package, -13 config sections of other programs, -14 user unit without `IPAddressDeny`, -15 doctor journal window.

### Scope and architecture
- Keyboard-only repository: the display, brightness, MQTT and Home Assistant code was removed; `apihub-app` keeps the keyboard tabs.
- Cargo workspace with a hardware-free core `akm-core`, a headless daemon `apple-kb-monitord` (single owner of the keyboard, `systemd --user`, D-Bus `com.agenceapi.AppleKbMonitor1`, BlueZ provider, tray), thin clients over D-Bus (`akmctl`, `apihub-app`, Plasma widget), event-driven actor on BlueZ signals.
- Rust is the only language of the driver: the interpreted CLI `apple-kb-monitor`, `apihub-settings`, their unit and tests were removed; `akmctl` covers `history` (+ `--since/--until/--last`, `export --csv`, `import`), `graph`, `waybar`, `metrics`, `led`, a safe `dump` (`0x47`/`0x46`/`0x49` only); one history file `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl`. The package no longer depends on an interpreter (3.1.0-6).
- Daemon D-Bus API v2: per-device objects `/devices/<MAC>`, `.Input`, `.Link`, `.Keymap`, `.Tray` interfaces, `StateChanged` and per-device signals, `Json` property.

### Battery
- The kernel `power_supply` node (what UPower reads) is the source of the displayed percentage, shown as "keyboard indication" and never rewritten; raw HID reports are telemetry.
- Real voltages `0x46` / `0x49` in mV in the history (schema 2); the former constant pseudo-voltage is marked `voltage_valid: false` and ignored.
- Charge **estimate** by battery chemistry (`[battery] chemistry`: alkaline / NiMH / lithium / unknown) with a range, grace period after a battery change, time remaining; alerts computed on the estimate when there is one. Battery-change detection and per-set log. Forecast in days.
- The keyboard's percentage only steps down at reconnections: the age of the last reading is shown everywhere and a drop seen on the first reading after a reconnection raises no alert.
- "Apple display" percentage (IOBluetooth curve, `[display] apple_percent`); firmware thresholds Full / Low / Critical / Empty (`0x60`) read once per connection, margins shown.
- Keyboard-driven alerts from Input `0x30` ("battery low / critical (keyboard alert)"), `BatteryAlert` signal, CapsLock flash on critical, deduplicated with the percentage alerts.
- Multi-threshold alerts with hysteresis (default 30 / 15 / 5 %), connection notifications, PowerDevil dedupe.
- Read cost: the daemon follows the safe read policy; UPower's 30 s polling is documented and optionally removed (`bluetooth/akm-conf.py upower`).

### Apple parity and register map
- Declarative register map `akm-core/src/registry.rs` of every known HID report (57 entries) with safety classes; read allow-lists generated from it; the hidraw doors ask it first; tests sweep the 256 ids in the three directions.
- Firmware version `0x4F` read once per connection and compared with an embedded, dated table; `akmctl firmware`, `akmctl info` (never reads the keyboard); D-Bus `Firmware*` properties.
- Pure model of Apple's macOS 26.5 driver (`apple_model.rs`, rules R1-R8 with sources): battery schedule 60 s then 4 h (1 h after a failure), GET Feature `0x47` then **GET Input `0x30`** then `0x46` / `0x49` 1 s apart, 3.5 s timeout, circuit breaker after 3 silences (2 after a sleep), a HANDSHAKE refusal resets the counter, BlueZ `Device1.Disconnect` after the third silence (`[apple] disconnect_on_breaker`), never `RemoveDevice` (the full breaker).
- The breaker reaches every emitter: published in `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state`, read by `akm-hid-control` (root) and by the `akmctl` write door; CapsLock flash blocked while open.
- `WillShutdown` (Feature `0x40`, id alone) sent once at shutdown / restart as macOS does, default on (`[apple] will_shutdown`): logind `PrepareForShutdown` inhibitor, `akmctl shutdown-notify`, user unit `apple-kb-monitor-shutdown.service`.
- Keyboard off (Input `0x13` bit 1 = 0) distinct from a link loss: `PoweredOff`, `KeyboardOff` signal, "switched off" notification.
- HID_CONTROL SUSPEND (`0x13`) before sleep and EXIT_SUSPEND (`0x14`) at wake, one byte on `bluetoothd`'s control socket (`pidfd_getfd`), root system units `apple-kb-monitor-suspend/resume.service`, `/etc/apple-kb-monitor/hid-suspend.conf`, `akmctl hid-control … --dry-run`, polkit action `hid-control`.
- Clean forget like macOS: `akmctl repair` on a connected keyboard sends SET Feature `0x41` `RecantConnection` after pre-flight, host-side backup and a typed `OUBLIER`, waits 2000 ms, then `RemoveDevice`; nothing removed if `0x41` fails.
- Name stored in the keyboard, first version: `akmctl rename --device-name <name> [--dry-run] [--write-device-name] [--show] [--restore]`; the Lion `setDeviceName:` frame (SET Feature `0x55`, 65 bytes) established by disassembly; the real write behind **three locks** (`[apple] allow_device_name_write = true`, control-channel MTU ≥ 66 read by `akm-hid-control inspect`, name typed again); D-Bus `DeviceNameOnKeyboard`, `akmctl status` line `On kb:`.
- Writes exist only as named operations (`Shutdown` `0x40`, `Forget` `0x41`, `DeviceName` `0x55`), each once per session with its exact length; one write function with two fixed sizes, every byte logged, never retried; a source scan fails on a second write path. Not implemented on purpose: `0x44` `FullFactoryDefault`, `0x4A` SCO notification.

### Link: reconnection, doctor, repair
- Root cause of the "keyboard does not reconnect" episodes measured and documented (radio silence under request bursts, BlueZ paging given up, adapter USB autosuspend); recovery state machine (connected, dormant, unreachable, auth-failed, suspended), paging cadence, logind sleep inhibitor.
- `akmctl doctor [--json]`: adapter, pairing, link, link key (sudo), hidraw, adapter power management, BlueZ `FastConnectable` / `Reconnect*`, UPower `NoPollBatteries`, `bluetoothd` journal over the last 6 h plus the boot (3.1.0-15), daemon; every finding with its fix. `akmctl repair [--force]`: wake + reconnect first, re-pairing only after a typed confirmation.
- `bluetooth/akm-conf.py {bluez,upower} [--apply]` and `udev/61-akm-bt-adapter-no-autosuspend.rules` (not installed by the package).
- `0xFE` is never requested (two link losses), `0x4C` never requested; a read requires a key press in the last 60 s outside the daemon (link losses).
- `Refresh()` bounded to one per 5 min; 1 s between requests; circuit breaker.

### Security
- udev rule `70-apple-kb-hidraw.rules` tags the hidraw node `uaccess` (logind ACL for the active seat), no `input` group; narrowed to the 17 Bluetooth product ids of the model table, wired Apple keyboards no longer matched; the keylogger trade-off is documented.
- `rssi-helper`: the only binary with `cap_net_admin`, applied by `post_install`; `root:akm 0750`, group `akm` created by sysusers.
- `SetFnMode` runs `/usr/bin/pkexec akm-helper set-fnmode <n>` from constants, caller uid checked, one dialog at a time; polkit `auth_admin` without `_keep`; `SetSwapOptCmd` / `SetIsoLayout` D-Bus methods removed. Plain-text keyboard names in the widget and escaped tray tooltip; aliases cannot start with `-`. Private per-uid `hid.lock` directory, `O_NOFOLLOW`. Systemd unit options compatible with pkexec and file capabilities; `IPAddressDeny` removed from the user unit (3.1.0-14). Dependencies updated, `deny.toml` documents the remaining advisories (the eframe 0.29 advisories stay tracked). Real `0x4C` fingerprints removed from fixtures.
- Sensitive bytes of `0x4C` never published.

### Keys
- keyd became optional: `keyd reload` crashes keyd 2.6.0 (reproduced, patch and bug report prepared), the package never reloads nor restarts it; the config is an example under `/usr/share/doc`.
- Special keys modelled from the kernel sources (udev hwdb → `hid_apple` → xkb / KDE): `akmctl keys [--check] [--all]`, Keys tab in the window, manual mapping without keyd (`keymap.toml` profiles and presets → validated udev hwdb installed by `pkexec akm-keymap-helper`, polkit action `install-keymap`), `akmctl keymap kde-apply` for F4 / Eject.
- `akmctl get/set fnmode|param` and `--persist` through `akm-helper`; Fn lock toggle from the tray. LEDs through evdev, via keyd's virtual keyboard when keyd grabs the device.

### KDE Plasma 6
- Tray in the daemon: StatusNotifierItem + dbusmenu, symbolic icons by level / charging / disconnected, rich tooltip and menu, re-registration when the watcher reappears. `apihub-app` single instance through D-Bus activation `com.agenceapi.AppleKbMonitor`, reverse-DNS desktop file, no autostart.
- Plasma widget `com.agenceapi.devicehub` driven by D-Bus signals, claims the notification area so there is one icon only; French catalogue.
- Window integrated with the desktop: Wayland `app_id`, theme / accent / contrast, French (156 strings); vsync, bounded D-Bus calls, history off the UI thread, heartbeat file.
- KNotification events (`apple-kb-monitor.notifyrc`, 14 events, buttons Open / Repair… / Ignore, replacement per slot, FR/EN). PowerDevil dedupe: one `BatteryEstimate` reminder when KDE already warns. Keyboard forgotten from Plasma removed at once, one `KeyboardRemoved` notification. `akmctl status` explains the two names (BlueZ alias vs kernel name).
- System Settings module "Apple Keyboard" (`kcm_applekeyboard`, C++ plugin + QML pages State, Keys, Notifications, Name, Diagnostics), no I/O on the GUI thread, writes `config.toml` atomically, uses the existing polkit actions and D-Bus methods only.
- Alias of the keyboard on this computer: `akmctl rename`, D-Bus `SetAlias`, tray, window, widget.

### Quality
- Local pipeline `scripts/ci-local.sh` (18 steps: versions, secrets, fmt, clippy, test, claims, redaction, deny, audit, udev, qml, shell, c, security, units, plasma, package, e2e), end-to-end tests of the window and daemon under Xvfb + bubblewrap with a simulated keyboard (11 scenarios + KCM), `akmctl selftest` every 15 min with notifications and optional tracker issues, a pre-push hook, CI with five jobs.
- Registry-backed claim check (`qa_checks.py claims`): documents and strings may not state the claims refuted by the counter-audit . Redaction check: no Apple binary or decompiled code committed.
- Fixtures of the real A1314 ISO, synthetic reports, Lion frames, other models; `tests/live/check_keyboard.sh` read-only hardware harness; tooled audits (miri, udeps, fuzz, proptest) and adversarial reviews in `docs/`.

### Packaging
- PKGBUILD with local sources, `!lto`, `backup=` for `hid_apple.conf` and `hid-suspend.conf`, dependencies declared for the dlopen'ed Wayland / X11 / GL libraries and the KDE module; `.SRCINFO` regenerated; desktop categories and D-Bus files fixed; widget launches the window.
- The package ships its unit links (`default.target.wants`, `suspend.target.wants`) and enables nothing with `systemctl`; a `pre_remove` helper disables only links it created under `/etc/systemd`. It re-applies the group and the capability of `rssi-helper` on every install / upgrade. Completions and manual generated by `akmctl` at build time.

### Fixed
- keyboard: HID fd leak, wake-monitor busy loop after disconnect, unvalidated calibration, PID substring match, NUL in device name, CapsLock LED; `power_supply` path without `-NN` suffix, null percentage without sysfs fallback; RSSI reply matched to its request, 127 rejected, strict MAC; BlueZ `PropertiesChanged`, re-registration, log flood; alerts re-armed, no 100 % / 0 V points, tray SNI retries, UTF-8 safe slicing; invalid history points rejected; no 0 % exported to BlueZ on a failed read.
- Daemon: no warning for the config sections of another program sharing `config.toml` (3.1.0-13).
- Final review of 3.1.0-19, fixed in 3.1.0-20:
  - a battery level kept from an earlier read (keyboard silent, read not due) is marked `battery.kept` and is no longer recorded as a new measure: no history sample, `last_update` unchanged; "keyboard silent" is said only when a read was really sent and failed;
  - after a wake, the battery read waits for the first key press and then goes out at once (the cycle is not consumed and retried 1 h later); without the passive listener nothing is held;
  - `RereadName` reads `0x51`-`0x54` alone (never the routine reports, `Refresh`'s 5 min floor is not bypassed), a request inside the 30 s floor is deferred instead of dropped, the method answers `(accepted, text)`, and the instant of the last hardware access is shared between `akmctl` and the daemon (`hid.last` next to `hid.lock`) so that the 1 s spacing holds across processes;
  - a keyboard whose current name is not ASCII (macOS writes UTF-8) can be renamed: the backup keeps the 32 raw bytes and the restore writes them back as they are; only a typed name must be ASCII;
  - with two Apple keyboards connected, the write is refused when the hidraw node opened is not the keyboard of the pre-flight and of the backup;
  - HID_CONTROL SUSPEND / EXIT_SUSPEND are off in the code too: `hid-suspend.conf` absent or without the `enabled` key means disabled; the upgrade warns when an edited file still says `enabled = true`;
  - polkit: the read-only MTU probe is its own executable `akm-hid-inspect`, so the password-less action `hid-inspect` is bound to its path alone (two actions on one path told apart by `argv1` depended on polkitd's enumeration order);
  - `akmctl rename --device-name`: exit code 15 "write uncertain" when the write failed on its way (a frame may have been sent), shown as such by the KCM instead of "Not written"; the command to copy from the KCM is `--device-name='<name>'` (a name starting with a dash was read as an option).

### Documentation
- README, INSTALL, CONFIGURATION, FEATURES, TROUBLESHOOTING, ARCHITECTURE, TESTING rewritten for 3.1.0-15, `docs/INDEX.md` added. Hardware reference documents (`HARDWARE-*.md`, `BATTERY-CHECK.md`) are dated evidence and are not rewritten.

### Known open items
- Hardware validation of 16 of the 17 models; multi-keyboard UI; Magic Keyboard battery report `0x90`; the effect on the keyboard of `WillShutdown`, `0x41` and the sleep / wake bytes is not observable; the name write (`0x55`) was never exercised on hardware. Open milestones: v3.2 backlog, v4.0 keyboard differentiation.

### Lot C: akmctl and packaging
- `config.toml` is read by the `toml` and `serde` crates instead of the hand-written line reader: any valid TOML is understood; same keys, defaults, warnings and `line N`; a file that is not valid TOML is still read statement by statement, one bad line costs one warning. Three spellings TOML forbids (`05`, `4.`, `[40,,20]`) now give a warning and the default. The JavaScript reader of the System Settings module (`kcm/ui/Toml.js`) is unchanged.
- PKGBUILD: `cargo fetch --locked` in `prepare()`, `cargo build --locked`, the real sha256 of the one source instead of `SKIP`, `.SRCINFO` checked against `makepkg --printsrcinfo` by `tests/check-pkgbuild.sh`. Still built from `$startdir`: no `git+` source and no clean-chroot build until a source repository is published.
- `/etc/modprobe.d/hid_apple.conf` stays in the package: it is the file `akmctl set fnmode N --persist` rewrites; the reasons are written in the file and checked in the package.
- `akmctl doctor --fix [--dry-run] [--restart-services] [--optional]`: the corrections of BlueZ `main.conf`, `UPower.conf` and the adapter udev rule are applied by a new helper `akm-doctor-fix` with its own polkit action `doctor-fix` (`auth_admin`): closed list, no path or value in argument, no shell, idempotent, previous file kept as `<name>.akm-bak`. Without `--fix`, `akmctl doctor` is unchanged.
- `akmctl keys --live`: live view of the key events (HID usage, evdev code, name), read-only on the evdev node, nothing recorded, left with Escape held 2 s or Ctrl-C.
- `docs/KEYS.md` §8: `evtest` procedure to prove that `<>` emits `KEY_102ND`, and a unit test of the `iso_layout` mapping. The measurement itself is still to be done on the keyboard.

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
