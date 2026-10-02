# Features

Every line below names the module that implements it (paths under `apihub-app/` unless stated) and its level of evidence: **[hardware]** observed on the real A1314 ISO (`05AC:0256`), **[code]** covered by tests and the simulated keyboard only, **[hypothesis]** when the meaning of a value is not established. Scope: the Apple Bluetooth keyboard only. The one-page summary is in the [README](../README.md#what-works).

## Battery and keyboard telemetry

| Feature | Evidence | Module |
|---|---|---|
| Percentage from the kernel `power_supply` node `hid-<mac>-battery*` (the one UPower reads, = Feature `0x47`), shown as "keyboard indication" and never rewritten | hardware | `akm-core/src/power.rs` |
| Voltage in mV: `0x46` (instant, u16 LE, cross-checked with `0xFF` BE), `0x49` (filtered); history schema 2 keeps both | hardware | `decode.rs`, `history.rs` |
| Charge **estimate** by chemistry (alkaline / NiMH / lithium / unknown) with a range, "new batteries" grace period, time remaining, battery-change detection and per-set log | code; curves = hypothesis from public datasheets | `chemistry.rs`, `forecast.rs`, `batteries.rs`, `snapshot.rs` |
| "Apple display" percentage (IOBluetooth remapping of the raw value) | code, from disassembly | `apple_model.rs`, `registry::apple_display_percent` |
| Keyboard-driven battery state, Input `0x30` (0 normal, 1 low, 2-3 critical): listened to passively **and** requested once per burst (`HIDIOCGINPUT`) as Apple's `getBatteryState` | hardware (`30 00` read) | `passive.rs`, `hidraw::hid_read_input` |
| Firmware thresholds Full / Low / Critical / Empty (`0x60`, 4 × u16 BE mV) read once per connection, margins shown | hardware | `registry.rs`, `decode.rs` |
| Firmware version `0x4F` read once per connection, compared with an embedded, dated table (`up_to_date` / `update_available` / `unknown`); no network, no flash | hardware (`0x0050`) | `firmware.rs`, [FIRMWARE.md](FIRMWARE.md) |
| Name stored in the keyboard (`0x51`-`0x54`), read once per connection, low priority | hardware | `devname.rs` |
| Register map of every known report: id, direction, size, Apple name, meaning, unit, endianness, decoder, proof, safety class; read allow-lists generated from it | — | `registry.rs`, `akmctl info [--json]` |
| 17 Apple models by product id (USB `05AC`, Bluetooth `004C`); raw reports read on the BCM2042 family only | 1 of 17 on hardware | `model.rs` ([HARDWARE-ENTREES-MODELES.md](HARDWARE-ENTREES-MODELES.md)) |

## Read policy and Apple parity

| Feature | Evidence | Module |
|---|---|---|
| Apple's schedule: first read 60 s after the connection, then every 4 h (1 h after a failure); burst `0x47`, Input `0x30`, `0x46`, `0x49` 1 s apart, 3 s budget, stop at the first failure | code (conformance tests R1-R8, proptest) | `apple_model.rs`, `read_policy.rs`, `machine.rs` |
| Circuit breaker: 3 consecutive silences stop every emission until a new connection or a sleep; a HANDSHAKE refusal is an answer; after a sleep 2 silences suffice | code | `read_policy::Breaker` |
| Breaker published in `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state` so that `akm-hid-control` and `akmctl` obey the daemon's verdict | code (fake `bluetoothd` test) | `breaker_state.rs` |
| BlueZ `Device1.Disconnect` once after the third silence, as macOS asks `bluetoothd` (`[apple] disconnect_on_breaker`) | code | `apple-kb-monitord/src/watcher.rs` |
| One reader at a time (process mutex + `flock` on `hid.lock`), shared with `akmctl dump` | code | `read_policy.rs` |
| `WillShutdown` (`0x40`) once at shutdown / restart: logind `PrepareForShutdown` inhibitor, `akmctl shutdown-notify`, user unit | code (effect not observable) | `parity.rs`, `apple-kb-monitord/src/sleep.rs`, `shutdown.rs` |
| HID_CONTROL SUSPEND `0x13` before sleep, EXIT_SUSPEND `0x14` at wake, one byte on `bluetoothd`'s control socket, root system units, `enabled` in `/etc/apple-kb-monitor/hid-suspend.conf` | code | `crates/akm-helper/src/hidctl.rs`, [VEILLE-HID.md](VEILLE-HID.md) |
| Keyboard off (Input `0x13` bit 1 = 0) distinct from a link loss: `PoweredOff`, `KeyboardOff` signal, "switched off" notification | hardware | `passive.rs`, `link.rs` |
| Passive input events: `0x04` sleep, `0x05` Fn lock, `0x11` Eject / Fn, `0x12` media bits, `0x13` wake; CapsLock / NumLock from sysfs | hardware | `passive.rs`, `apple-kb-monitord/src/passive.rs` |
| What is **never** done: GET on `0x4C` (pairing record) and `0xFE` (froze the firmware twice); any write outside the three named operations | tests sweep the 256 ids | `registry.rs`, `hidraw.rs` |

## Link: diagnosis and repair

| Feature | Evidence | Module |
|---|---|---|
| Reconnection state machine (connected, dormant, unreachable, auth-failed, suspended), paging cadence, logind sleep inhibitor | hardware (episodes of 2026-10-01) | `recovery.rs`, `apple-kb-monitord/src/sleep.rs`, [RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) |
| `akmctl doctor [--json]`: adapter powered, pairing / bonding, link, link key (with sudo), hidraw readable, adapter USB autosuspend, BlueZ `FastConnectable` / `Reconnect*`, UPower `NoPollBatteries`, `bluetoothd` journal (6 h + boot), daemon; each finding carries a fix command | hardware | `crates/akmctl/src/doctor.rs` |
| `akmctl repair [--force]`: wake + reconnect first; re-pairing only after a typed confirmation; clean forget of a **connected** keyboard = SET `0x41` `RecantConnection`, 2000 ms, `RemoveDevice` (typed `OUBLIER`, host-side backup without link key, skipped when the breaker is open) | code (effect of `0x41` not measured) | `repair.rs`, `forget.rs`, `apple-kb-monitord/src/repair.rs` |
| Keyboard removed from outside (Plasma "Forget", `bluetoothctl remove`): removed at once from the daemon, one `KeyboardRemoved` notification with "Repair…" | hardware (#252) | `apple-kb-monitord/src/devices.rs` |
| Recommended BlueZ / UPower settings tool and adapter autosuspend udev rule | hardware (Intel `8087:0026`) | `bluetooth/akm-conf.py`, `udev/61-akm-bt-adapter-no-autosuspend.rules` |
| RSSI / TX power through `rssi-helper` (BlueZ MGMT `GET_CONN_INFO` `0x0031`, `cap_net_admin`, `root:akm`), 1.5 s timeout, 10 s cache; shown as a relative quality (excellent / good / weak), never as a power level | hardware | `rssi.rs`, `signal.rs`, `rssi-helper.c` |
| Tray Bluetooth actions: "Reconnect" (`Device1.Connect`, 20 s spacing of the recovery machine), "Disconnect" (`Device1.Disconnect`, no page until asked), "Forget this keyboard…" (only asks: `ForgetConfirm` notification, one button valid 60 s, then `Adapter1.RemoveDevice`); `Link.Disconnect`, `Link.RequestForget` (#104) | code (fake BlueZ + fake notification server on a private bus) | `apple-kb-monitord/src/repair.rs`, `forget.rs`, `akm-core/src/recovery.rs` |
| Link quality: disconnections with their reason, counts per hour / day, 7 days of relative signal, "unstable link" alert above 3 disconnections within an hour, once per episode; `link_quality` in `GetState`, `Link.Quality()`, `akmctl doctor` line (#105) | code (simulated clock) | `akm-core/src/linkstats.rs`, `apple-kb-monitord/src/linkq.rs` |
| Several keyboards: `devices` in `GetState`, one D-Bus object per keyboard, tray on the weakest connected one; Fn mode remembered per address and offered back at reconnection (#94, #119, #103) | code (two simulated keyboards) | `akm-core/src/roster.rs`, `device_settings.rs`, `apple-kb-monitord/src/reapply.rs` |
| `Diagnose()`: the checks of the window's Diag tab as JSON, by the daemon (#120) | code (fixture tree) | `apple-kb-monitord/src/diagnose.rs` |

## Desktop (KDE Plasma 6)

| Feature | Evidence | Module |
|---|---|---|
| Tray icon in the daemon: StatusNotifierItem + dbusmenu, symbolic icons by level / charging / disconnected, tooltip (percentage, estimate, age of the reading, firmware), menu "Rename keyboard…", "Copy information", "Fn mode" (state read in sysfs, two radio items, `Device.SetFnMode` + polkit), "Quit (hide icon)"; re-registers when the StatusNotifierWatcher reappears | hardware | `apple-kb-monitord/src/tray/` |
| KNotification events (`apple-kb-monitor.notifyrc`, 14 events: `BatteryLow`, `BatteryCritical`, `KeyboardAlert`, `KeyboardDisconnected`, `KeyboardReconnected`, `KeyboardOff`, `KeyboardUnreachable`, `RepairNeeded`, `KeyboardRemoved`, `BatteryEstimate`, `FirmwareUpdate`, `BatteryReminder`, `BatteryReplaced`, `Error`), buttons Open / Repair… / Ignore, replacement per slot, FR/EN; `FirmwareUpdate` and `BatteryReminder` have no daemon trigger yet | hardware | `apple-kb-monitord/src/notify.rs`, [NOTIFICATIONS.md](NOTIFICATIONS.md) |
| PowerDevil deduplication: one `BatteryEstimate` reminder when PowerDevil already warns (`[notifications] defer_to_powerdevil`) | hardware (#254) | `apple-kb-monitord/src/powerdevil.rs` |
| Plasma widget `com.agenceapi.devicehub` ("ApiHub"): compact + full views, keyboard name, battery, estimate, firmware, signal, last reading, 7-day sparkline, Fn mode with a toggle button (`Device.SetFnMode`, polkit in the daemon); popup pages History (7 / 30 / 90 days, battery and voltage) and Diagnostics (daemon, link, firmware; Read again / Reconnect), from the existing D-Bus methods; claims the notification area (one icon); FR catalogue | hardware; sparkline, Fn button and pages: headless tests only | `plasma/com.agenceapi.devicehub/` |
| System Settings module "Apple Keyboard" (`kcm_applekeyboard`): pages State, Keys, Notifications, Name, Diagnostics; no I/O on the GUI thread; writes only on click, through the existing polkit actions and D-Bus methods | hardware | `kcm/`, [KCM.md](KCM.md) |
| Window `apihub-app` (egui, tabs Keyboard with history graph over 24 h / 7 / 30 / 90 days (at most 720 points drawn per series, whatever the size of the history), Keys, Diag), D-Bus activatable `com.agenceapi.AppleKbMonitor`, single instance, no tray icon of its own (the icon belongs to the daemon), French (177 strings), heartbeat file for the self-check, bounded D-Bus calls | hardware + e2e (Xvfb) | `apihub-app/src/` |
| Global shortcuts (KGlobalAccel component "Apple Keyboard": open the window, toggle the function keys), no key bound by default; command `apihub-app --toggle-fn` (`Device.SetFnMode` through the daemon) | code + private-bus tests; not tried in a live Plasma session | `data/com.agenceapi.AppleKbMonitor.shortcuts.desktop`, `apihub-app/src/fn_toggle.rs`, [INTEGRATION-KDE.md](INTEGRATION-KDE.md) §9 |
| KRunner runner ("clavier", "keyboard", "batterie clavier", "fn lock"): battery level and autonomy, open the window, open the settings module, toggle the function keys; D-Bus-activated `apihub-app --krunner`, exits after 2 min idle | code + private-bus tests; not tried in a live Plasma session | `data/plasma-runner-applekeyboard.desktop`, `apihub-app/src/krunner.rs`, [INTEGRATION-KDE.md](INTEGRATION-KDE.md) §10 |
| BlueZ `BatteryProvider1` per keyboard (`org.bluez.Battery1` fallback when the kernel has no node; UPower hides the duplicate otherwise), re-registration when `bluetoothd` restarts | code | `apple-kb-monitord/src/bluez.rs` |
| Alias on this computer (BlueZ `Alias`): `akmctl rename`, D-Bus `SetAlias`, tray, window, widget, module; the kernel name changes only after a reconnection (explained by `akmctl status`) | hardware | `alias.rs`, `apple-kb-monitord/src/alias.rs` |
| Notifications: quiet hours (`[notifications] quiet_hours`, critical always shown, held ones journalled and shown at the end), "Remind me tomorrow" (24 h, persistent), "batteries changed too often" after two sets under 30 days (#91, #110, #108) | code (simulated clock, fake notification server) | `akm-core/src/quiet.rs`, `deferred.rs`, `advice.rs`, `apple-kb-monitord/src/notify_policy.rs` |
| Plasma OSD (`org.kde.osdService`) at a change of Fn mode / layout and at a Caps Lock press, Caps Lock badge on the tray icon (#100, #101) | code (fake plasmashell) | `apple-kb-monitord/src/osd.rs` |
| Usage statistics without key logging, off by default: active minutes per day (#109) | code (key reports through a pipe leave only a counter) | `akm-core/src/activity.rs`, `usage.rs`, `apple-kb-monitord/src/usage.rs` |

## Keys

| Feature | Evidence | Module |
|---|---|---|
| Effective table of the special keys: physical key → evdev code (after hwdb and `hid_apple`, per `fnmode`) → keysym → KDE global shortcut; `akmctl keys [--check] [--all]`, Keys tab, module page | hardware | `keymap.rs`, `keytable.rs`, `keycodes.rs`, [TOUCHES.md](TOUCHES.md) |
| Fn mode and `hid_apple` parameters (`fnmode`, `iso_layout`, `swap_opt_cmd`, `swap_ctrl_cmd`, `swap_fn_leftctrl`) through `pkexec akm-helper`, persisted in `/etc/modprobe.d`; `akmctl get/set fnmode`, `get/set param`, D-Bus `SetFnMode` | hardware | `hid_params.rs`, `crates/akm-helper/` |
| Manual mapping without keyd: `keymap.toml` profiles and presets (`apple`, `fkeys`, `linux-pc`, `none`) → validated udev hwdb installed by `pkexec akm-keymap-helper`; `rollback`, `reset` | hardware | `keymap.rs`, D-Bus `Keymap` |
| `akmctl keymap kde-apply [--dry-run|--undo]`: adds the KDE shortcuts the Apple legend needs (F4 → application launcher), never replacing a binding | hardware | `crates/akmctl/src/kde.rs` |
| LEDs through evdev `EV_LED` (via keyd's virtual keyboard when keyd grabs the device); NumLock is never switched on | code | `led.rs`, `akmctl led` |
| keyd optional: example config in `/usr/share/doc`, never reloaded by the package | hardware (keyd 2.6.0 crash reproduced) | `keyd/`, `tests/keyd/`, [KEYD.md](KEYD.md) |

## CLI `akmctl`

The only command-line tool; everything but `dump`, `led`, `rename --device-name` (write and read-back) and `repair`'s forget goes through the daemon on the session bus. Exit codes: 0, 1 error, 2 daemon absent, 64 usage. `akmctl completions {bash,zsh,fish}` and `akmctl man` generate the files shipped by the package.

| Command | Role |
|---|---|
| `status [--json]`, `watch` | state (JSON schema 1: battery, estimate, Apple display, thresholds, firmware, passive events, signal, Fn mode, names); one JSON line per `StateChanged` |
| `history [--since W] [--until W] [--last N] [--json]`, `history export --csv`, `history import [FILE]` | single store `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl`; `W` = `90m`, `24h`, `7d`, `2w`, `YYYY-MM-DD`, `"YYYY-MM-DD HH:MM"` (UTC) |
| `graph [--span 24h\|7d]`, `waybar`, `metrics` | terminal chart; waybar JSON (classes `good` > 50 %, `warning` 16-50 %, `critical` ≤ 15 %, `disconnected`); Prometheus text (`apple_kb_*`) |
| `get/set fnmode`, `get/set param` | `hid_apple` parameters, polkit for `set` |
| `rename [NAME] [--reset] [--mac]`, `rename --device-name [NAME] [--yes] [--check] [--dry-run] [--verbose] [--show] [--restore BACKUP]` | alias on this computer / name stored in the keyboard: written after one confirmation (`--yes` skips it), read back at once; exit codes 10 no confirmation possible, 11 pre-flight, 12 cancelled, 13 not read back, 14 read back different |
| `keys [--check] [--all]`, `keymap {show,set,unset,preset,use,apply,reset,rollback,kde-apply}` | special keys |
| `led <caps\|num\|scroll\|compose\|kana> <on\|off>` | evdev LED |
| `info [--json]`, `firmware [--json]` | register map with cached values; firmware status |
| `dump [--json]` | the 3 allowed reports `0x47`, `0x46`, `0x49` under the safe read policy (press a key first) |
| `doctor [--json] [--mac]`, `repair [--force] [--mac]` | link diagnosis and guided repair |
| `shutdown-notify [--only-if-stopping]`, `hid-control {suspend,exit-suspend} [--mac] [--dry-run]` | the shutdown unit's command; manual test of the sleep / wake byte (polkit) |
| `selftest [--json] [--notify] [--gitea-issue] [--state-file] [--no-save]` | the 15-minute health check |

Waybar: `"custom/kb": {"exec": "akmctl waybar", "return-type": "json", "interval": 60}`. Prometheus: `akmctl metrics > /var/lib/node_exporter/textfile/apple_kb.prom`.

## D-Bus API (session bus, `com.agenceapi.AppleKbMonitor1`)

Object `/com/agenceapi/AppleKbMonitor1` (`busctl --user tree com.agenceapi.AppleKbMonitor1`): properties `Battery`, `Voltage`, `Rssi` (127 = unknown), `Connected`, `Model`, `Mac`, `Name`, `LastUpdate`, `LastError`, `FirmwareVersion`, `FirmwareLatestKnown`, `FirmwareStatus`, `DeviceNameOnKeyboard`, `RemainingSeconds`, `Revision`, `Json`, `InterfaceVersion`, `DaemonVersion`; methods `Diagnose` (#120), `HistoryMax(t since, u max)` (#96; `History` returns at most 2000 points), `GetState` (also `devices`, `link_quality`, `battery_advice`, `usage`), `Refresh` (bounded to one per 5 min), `RereadName` → `(b accepted, s text)` (after `akmctl rename --device-name`: forget `0x51`-`0x54` and read these four again, never the routine reports; now, or at the end of a 30 s floor: deferred, never dropped; `false` while disconnected; writes nothing), `NotifyShutdown`, `SetAlias`, `History(since)`, `GetDevices`, `BatterySets`, `ExpectDisconnect`; signal `StateChanged`. The same object carries `.Input` (passive events: `Listening`, `FnLock`, `LastSleepEvent`, `WakeCount`, `EjectPressed`, `FnPressed`, `PoweredOff`; signals `SleepEvent`, `Wake`, `EjectChanged`, `FnLockUpdated`, `KeyboardOff`, `BatteryAlert`) and `.Keymap` (`KeyTable`, `Keymap`, `SetKey`, `SetPreset`, `UseProfile`, `Apply`, `Reset`). Children: `/devices/<MAC with _>` implements `.Device` (telemetry plus `DischargeRate`, `EmptyAt`, `BatteriesInstalledAt`, method `SetFnMode`, signals `BatteryLevelCrossed`, `ConnectionChanged`, `BatteryReplaced`), `/Link` (`Status`, `Reconnect`), `/Tray`. `busctl --user introspect com.agenceapi.AppleKbMonitor1 /com/agenceapi/AppleKbMonitor1` lists the exact signatures.

## Quality

| Feature | Module |
|---|---|
| Self-check every 15 min (`apple-kb-monitor-selfcheck.timer`): daemon, restarts, freshness, versions, link, journals, panics, coredumps, window heartbeat and CPU, disk, history; desktop notification for a new grave problem, optional Gitea issue | `crates/akmctl/src/selftest.rs`, [QA-AUTOMATIQUE.md](QA-AUTOMATIQUE.md) |
| Local pipeline `scripts/ci-local.sh` (18 steps), end-to-end window tests under Xvfb + bubblewrap, pre-push hook, Gitea Actions | [TESTING.md](TESTING.md) |
| Registry-backed claim check: documents and strings are scanned for refuted claims (`scripts/qa_checks.py claims`) | `scripts/qa_checks.py`, [CONTRE-AUDIT.md](CONTRE-AUDIT.md) |

## Not done

Tracked in Gitea: multi-keyboard UI ([#119](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/119)), Magic Keyboard battery through report `0x90` ([#95](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/95)), hardware validation of the 16 other models ([#23](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/23)), SCO coexistence `0x4A` ([#216](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/216), refused), the v3.2 and v4.0 milestones ([milestones](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/milestones)).
