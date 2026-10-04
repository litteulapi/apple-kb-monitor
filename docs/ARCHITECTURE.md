# Architecture

Module headers in the source are the reference for design decisions; this document is the map. Size, in rounded `wc -l` counts (tests included, October 2026): `akm-core/src` about 26 k lines, `apple-kb-monitord/src` 17 k, `crates/akmctl/src` 12 k, `crates/akm-helper/src` 4 k, `kcm/` 2.6 k QML, Plasma widget 3.4 k QML/JS.

## Principle

One **user daemon** owns the keyboard. Everything else is a client of its D-Bus interface or a tiny privileged helper that does exactly one thing. The hardware is addressed through a **declarative register map** (`akm-core/src/registry.rs`) and a **model of Apple's own driver** (`akm-core/src/apple_model.rs`): what is read, when, how often, and what the three possible writes are, is decided by data that cites its source, not by scattered code.

```
            session bus (D-Bus)                           system bus
 ┌────────────────────────────────────────────┐    ┌──────────────────────────┐
 │ apple-kb-monitord  (user service, Rust)    │    │ bluetoothd (BlueZ)       │
 │  com.agenceapi.AppleKbMonitor1             │◄──►│  Device1, Adapter1,      │
 │   .Device  .Input  .Link  .Keymap  .Tray   │    │  BatteryProviderManager1 │
 │  widget menu (.Tray), KNotification        │    └──────────────────────────┘
 │  akm-core: registry, read_policy, breaker, │              ▲
 │  apple_model, history, chemistry, keymap   │              │ pidfd_getfd, 1 byte
 └───────┬───────────────┬────────────────────┘    ┌─────────┴────────────────┐
         │ hidraw GET     │ child, cap_net_admin    │ akm-helper hid-control   │
         ▼               ▼                          │ sleep / wake units       │
 /dev/hidrawN      rssi-helper (MGMT)              └──────────────────────────┘
 /sys/class/power_supply/hid-<mac>-battery*
         ▲
 clients ├─ akmctl (CLI)        ├─ Plasma widget (notification area)
         ├─ kcm_applekeyboard (System Settings)
         └─ pkexec akm-helper set-fnmode | install-keymap | hid-control | hid-inspect | doctor-fix
```

## Components

| Component | Language, place | Privilege | Role |
|---|---|---|---|
| `akm-core` | Rust library, `apihub-app/akm-core/` | none (pure logic + the two hidraw doors) | model table, register map, read policy and circuit breaker, Apple model, decoders, history, chemistry and forecast, alerts, keymap, recovery state machine, config parser. Testable without hardware |
| `apple-kb-monitord` | Rust binary, `apihub-app/apple-kb-monitord/`, unit `apple-kb-monitord.service` (user, `Type=dbus`) | user of the active seat (`uaccess`) | the only process that opens the keyboard; D-Bus service; BlueZ battery provider; widget menu; notifications; logind sleep / shutdown inhibitors; PowerDevil dedupe; repair keeper |
| `akmctl` | Rust binary, `apihub-app/crates/akmctl/` | user; `pkexec` for `set`, `keymap apply`, `hid-control` | CLI: D-Bus client, doctor, repair, selftest, keys / keymap, outputs; the only client that may open the keyboard itself (`dump`, `led`, the two write commands under the shared lock) |
| `akm-helper` | Rust, `apihub-app/crates/akm-helper/` | root through `pkexec`; `argv[1]` selects the polkit action (`exec.argv1`) | `set-fnmode`: `hid_apple` parameters (`/sys/module/hid_apple/parameters/*`, `/etc/modprobe.d/hid_apple.conf`); `install-keymap`: validates and installs `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb`; `hid-control`: one byte `0x13`/`0x14` on `bluetoothd`'s L2CAP control socket (also run by the sleep / wake system units); `hid-inspect` (`allow_active = yes`): read-only MTU of that socket; `doctor-fix`: corrections of `akmctl doctor` |
| `rssi-helper` | C, `rssi-helper.c` | file capability `cap_net_admin+ep`, `root:root 0755`, answers only for a connected Apple keyboard | BlueZ MGMT `GET_CONN_INFO` (`0x0031`): RSSI, TX power; JSON on stdout; run as a child by the daemon (1.5 s timeout, 10 s cache) |
| `kcm_applekeyboard` | C++ plugin + QML pages, `kcm/` | user | System Settings module: State, Keys, Notifications, Name, Diagnostics; asynchronous D-Bus calls only; settings saved through the daemon's `Settings` interface (`RunAkmctl` for `akmctl`) |
| Plasma widget | QML, `plasma/com.agenceapi.devicehub/` | user | the only icon: compact / full views on the session bus (`DaemonLink.qml`), right-click menu from `…1.Tray`, translated catalogues |
| System files | `systemd/`, `dbus/`, `polkit/`, `udev/`, `modprobe/`, `data/`, `keyd/` | — | units, D-Bus activation files, polkit policy, `uaccess` rule, `fnmode=1`, `notifyrc`, keyd example |

## `akm-core`: what decides

- **`model.rs`**: `APPLE_MODELS`, 17 product ids from the kernel `hid-ids.h` (vendors `05AC` and `004C`), families `Bcm2042`, `MagicKeyboard`, `Unknown`; raw reports only on `Bcm2042`.
- **`registry.rs`**: every known report (57 entries: Feature, Input, Output) with direction, size, Apple name, meaning, unit, endianness, decoder, proof and safety class (`SafeRead`, `SafeReadInput`, `OncePerConnection`, `PassiveInput`, `ManualOnly`, `NeverRead`, `WriteApple`, `NeverWrite`, `Unknown`). Read allow-lists are generated from it. Writes exist only as **named operations** (`WriteOp::Shutdown` = `0x40`, `Forget` = `0x41`, `DeviceName` = `0x55`), each with an exact length, once per `WriteSession`.
- **`hidraw.rs`**: the two GET doors (`hid_read_feature` → `HIDIOCGFEATURE`, `hid_read_input` → `HIDIOCGINPUT`, one `ioctl` call `hid_get_report`) and the single SET door `hid_write_feature` with two fixed sizes (1 byte: `Shutdown`, `Forget`; 65 bytes: `DeviceName`), every byte logged, never retried. `WriteDoor` is the same door for a short-lived `akmctl` under the shared lock, obeying the daemon's published breaker. A source scan test fails if a second write path appears.
- **`apple_model.rs`**: pure state machine of the macOS 26.5 driver (rules R1-R8 of [APPLE-PARITY.md](APPLE-PARITY.md)): readiness, battery schedule (60 s, 4 h, 1 h after failure), request spacing (1 s), timeouts (3.5 s + 1 s watchdog), breaker (3 silences; 2 after a sleep), disconnection request, display curve, battery state, keyboard off, `WillShutdown`. One constants table `APPLE`, every field with its source.
- **`read_policy.rs`**: applies the model to the hidraw node: routine burst `0x47`, Input `0x30`, `0x46`, `0x49`; once-per-connection `0x4F`, `0x60`, `0x51`-`0x54`; process mutex + `flock` on `$XDG_RUNTIME_DIR/apple-kb-monitor/hid.lock`; `Breaker`; `publish_breaker_state` → `breaker_state.rs` (`breaker.state`, rewritten on change or every 20 s, owner / symlink / size checked by the root readers).
- **`machine.rs`**, **`link.rs`**, **`recovery.rs`**: connection lifecycle (first acquisition = readiness), passive events, reconnection episodes (connected, dormant, unreachable, auth-failed, suspended) and paging cadence.
- **`decode.rs`**, **`passive.rs`**, **`report.rs`**: decoders of the Feature values and of the Input reports `0x04`, `0x05`, `0x11`, `0x12`, `0x13`, `0x30`.
- **`power.rs`**: kernel `power_supply` resolution through `HID_UNIQ`, strict MAC validation, parsers tested on a fake tree.
- **`history.rs`**, **`chemistry.rs`**, **`forecast.rs`**, **`batteries.rs`**, **`snapshot.rs`**, **`alerts.rs`**: JSONL store (schema 2), discharge curves, estimate and range, battery-change detection, time remaining, thresholds with hysteresis and dedupe.
- **`firmware.rs`**, **`devname.rs`**, **`alias.rs`**: embedded firmware table; validation, backup and three-lock sequence of the name stored in the keyboard; BlueZ alias.
- **`keymap.rs`**, **`keytable.rs`**, **`keycodes.rs`**, **`hid_params.rs`**: the three layers of a key (udev hwdb → `hid_apple` → xkb / KDE), `keymap.toml`, presets, hwdb whitelist validator, `hid_apple` parameter whitelist.
- **`rssi.rs`**, **`signal.rs`**, **`led.rs`**, **`config.rs`**, **`calibration.rs`**, **`parity.rs`**: helper runner and relative signal quality; evdev LEDs; `config.toml`; `0x5A` table validation; `WillShutdown` emission.

## `apple-kb-monitord`: what runs

`main.rs` → `service.rs` exports `com.agenceapi.AppleKbMonitor1` at `/com/agenceapi/AppleKbMonitor1` (`Type=dbus`, `BusName=`). `actor.rs` is the single event loop: BlueZ signals (`watcher.rs`), hidraw reads under the policy, passive input listener (`passive.rs`, interface `.Input`), alerts and notifications (`notify.rs`, KNotification dialect, `powerdevil.rs`), per-device objects (`devices.rs`, `/devices/<MAC>`), keymap interface (`keymap.rs`), repair keeper (`repair.rs`, `/Link`), BlueZ provider (`bluez.rs`, `org.bluez.BatteryProvider1` under `/com/agenceapi/AppleKbMonitor`, re-registered when `bluetoothd` restarts), widget menu (`tray.rs`, `tray/view.rs`, `tray/actions.rs`, `/Tray`: `MenuItems`, `ActivateMenuItem`, `OpenPanel`), sleep and shutdown inhibitors (`sleep.rs`, `shutdown.rs`), aliases (`alias.rs`), settings (`settings.rs`, `pkexec akm-helper`). `client.rs` is the D-Bus client of `apple-kb-monitord --json` and of the tests (`akmctl` has its own, `crates/akmctl/src/bus.rs`).

Around that loop: `notify_policy.rs` decides WHEN a notification is shown (quiet hours, "Remind me tomorrow"; persistent queue `akm-core::deferred`); `osd.rs` shows the Plasma OSD (Fn mode, Caps Lock) from sysfs values only; `linkq.rs` keeps the link quality written by the link keeper (disconnections) and the actor (signal); `forget.rs` is the confirmation gate of the menu's "Forget", the only sender of the keeper's `UserForget`; `reapply.rs` remembers the Fn mode per keyboard; `usage.rs` counts active minutes from `akm_core::activity::note()`, a call without argument; `diagnose.rs` answers `Diagnose()`. None of them opens the hidraw node or sends anything to the keyboard. `testbus.rs` gives the tests a `dbus-daemon` without service directory.

Unit hardening (`systemd/apple-kb-monitord.service`): `UMask=0077`, `StateDirectory=apple-kb-monitor`, `KeyringMode=private`, `LimitCORE=0`, `TasksMax=128`, `MemoryMax=512M`, `Restart=on-failure`, `UnsetEnvironment=WAYLAND_DISPLAY DISPLAY` (the daemon never opens a window). The system units of `akm-helper hid-control` are `ProtectSystem=strict`, `CapabilityBoundingSet=CAP_SYS_PTRACE CAP_DAC_READ_SEARCH`, `SystemCallFilter=@system-service pidfd_getfd`, `MemoryDenyWriteExecute=yes`.

## Data flow

```
Hardware / kernel
  Apple keyboard (BT HID) --> /dev/hidrawN            GET Feature 0x47 0x46 0x49 0x4F 0x60 0x51-0x54, GET Input 0x30,
                                                       passive Input 0x04 0x05 0x11 0x12 0x13 0x30; SET 0x40 / 0x41 / 0x55 (named ops)
  hid-apple / hid-input   --> /sys/class/power_supply  battery percentage and status (source of the displayed %)
  BlueZ (D-Bus)           <-> Device1, Adapter1        connection state, Alias, Disconnect (breaker), RemoveDevice (repair only)
  BlueZ MGMT (HCI ctl)    <-- rssi-helper              RSSI, TX power
  bluetoothd L2CAP ctl    <-- akm-helper hid-control        HID_CONTROL 0x13 / 0x14 at sleep / wake

System layer
  udev 70- uaccess        ACL on hidraw for the active seat user (17 Bluetooth product ids)
  udev hwdb 90-           optional key mapping written by akm-helper install-keymap
  hid_apple               fnmode and swaps (akm-helper)
  polkit                  set-fnmode, install-keymap, hid-control (auth_admin, local active session)
  logind                  PrepareForSleep / PrepareForShutdown delay inhibitors

Desktop layer
  KDE / UPower            kernel battery (and BlueZ Battery1 when no kernel node); PowerDevil warnings observed
  KNotification           apple-kb-monitor.notifyrc events, actions, replacement
  Plasma widget, KCM, akmctl                 D-Bus com.agenceapi.AppleKbMonitor1 (+ Json property, StateChanged)
```

## Privilege model

| Component | Privilege | Mechanism |
|---|---|---|
| hidraw read / write (named ops) | user of the active seat | udev `uaccess` (`70-apple-kb-hidraw.rules`, sorts before `73-seat-late.rules`); accepted keylogger trade-off in [../udev/README.md](../udev/README.md) |
| sysfs `power_supply`, `hid_apple` parameters (read) | any user | world-readable attributes |
| BlueZ D-Bus | unprivileged | standard BlueZ policy |
| RSSI (MGMT) | `cap_net_admin` | file capability on `rssi-helper`, which answers only for a connected Apple keyboard |
| `hid_apple` write, `/etc/modprobe.d` | root | `pkexec akm-helper`, action `set-fnmode`, `auth_admin` without `_keep` |
| udev hwdb | root | `pkexec akm-helper install-keymap`, action `install-keymap`, whitelist validator |
| HID_CONTROL byte | root | system units at sleep / wake; `pkexec akm-helper hid-control`, action `hid-control`, for the manual test |

## File layout (installed)

```
/usr/bin/                                  akmctl, apple-kb-monitord
/usr/lib/apple-kb-monitor/                 rssi-helper (cap_net_admin, 0755), akm-helper
/usr/lib/systemd/user/                     apple-kb-monitord.service, apple-kb-monitor-shutdown.service, apple-kb-monitor-selfcheck.{service,timer}
/usr/lib/systemd/system/                   apple-kb-monitor-suspend.service, apple-kb-monitor-resume.service
/usr/lib/udev/rules.d/                     70-apple-kb-hidraw.rules
/usr/lib/qt6/plugins/plasma/kcms/systemsettings/   kcm_applekeyboard.so
/usr/share/dbus-1/services/                com.agenceapi.AppleKbMonitor1.service
/usr/share/polkit-1/actions/               com.agenceapi.AppleKbMonitor.policy
/usr/share/knotifications6/                apple-kb-monitor.notifyrc
/usr/share/plasma/plasmoids/               com.agenceapi.devicehub/
/usr/share/applications/                   kcm_applekeyboard.desktop
/usr/share/kglobalaccel/                   com.agenceapi.AppleKbMonitor.shortcuts.desktop
/usr/share/icons/hicolor/scalable/         apps/apihub-scarab.svg, status/apihub-kb-*.svg
/usr/share/doc/apple-kb-monitor/           KCM.md, KEYD.md, TESTING.md, KEYS.md, HID-SLEEP.md, udev-README.md, examples/keyd/
/etc/apple-kb-monitor/hid-suspend.conf     enabled = false (SUSPEND off by default)
/etc/modprobe.d/hid_apple.conf             fnmode=1
~/.config/apple-kb-monitor/                config.toml, keymap.toml, selfcheck.env
~/.local/state/apple-kb-monitor/           history.jsonl, selfcheck.json, *-backup-<UTC>.json
$XDG_RUNTIME_DIR/apple-kb-monitor/         breaker.state, hid.lock, keymap.hwdb (staged)
```

## Repository layout

```
apihub-app/              Cargo workspace (single version source: [workspace.package] version = PKGBUILD pkgver = CHANGELOG)
  akm-core/              library + tests/ (integration tests)
  apple-kb-monitord/     daemon + tests/
  crates/akmctl/         CLI + tests/cli.rs (the built binary on fixtures)
  crates/akm-helper/     akm-helper (one binary, one command per polkit action)
  assets/fonts/          OFL licence of the widget font
kcm/                     System Settings module (CMake), captures/
plasma/                  widget, po/*.po, tests/ (qml6 tests on a fake daemon, catalogue checks)
data/ dbus/ polkit/ systemd/ udev/ modprobe/ keyd/   system integration
po/                      catalogues of the daemon and akmctl
scripts/                 ci-local.sh, qa_checks.py, package-expected.txt, update-po.sh
tests/                   check-*.sh, e2e/ (KCM and widget under Xvfb, private session bus), fixtures/ (real A1314 capture, synthetic, devname, models), live/ (read-only tools), keyd/
docs/                    index: INDEX.md
.github/workflows/       ci.yml (checks), package.yml (Arch package, releases)
```

