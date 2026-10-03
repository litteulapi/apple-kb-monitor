<p align="center">
  <h1 align="center">apple-kb-monitor</h1>
  <p align="center">
    Battery, link and key monitor for Apple Wireless Keyboards on Linux (KDE Plasma 6).<br>
    A user daemon that reads the keyboard the way Apple's own driver does, and nothing more.
  </p>
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-2021-DEA584?logo=rust&logoColor=black" alt="Rust"></a>
  <a href="https://kernel.org/"><img src="https://img.shields.io/badge/Platform-Linux-FCC624?logo=linux&logoColor=black" alt="Linux"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--2.0--or--later-blue" alt="GPL-2.0-or-later"></a>
</p>

---

Package **3.1.0-15** (Arch Linux / Manjaro). Tested on one keyboard: Apple Wireless Keyboard **A1314** aluminium ISO (`05AC:0256`, BCM2042, firmware `0x0050`). French and English.

## What it does

The Linux kernel already shows the percentage of these keyboards (`power_supply` node, read by UPower). This project adds what the kernel does not give:

- the **real battery voltage** (mV) and an **estimate of the remaining charge** by battery chemistry, next to the keyboard's own percentage, which is only a firmware interpolation that steps down at reconnections;
- the **firmware version**, the **name stored in the keyboard**, the keyboard-driven **low / critical battery alerts** (Input report `0x30`), the **keyboard off** event, the **special keys** table;
- a **link doctor and a guided repair** for the classic "the keyboard does not reconnect" problem;
- a complete **KDE Plasma 6 integration**: tray icon, KNotification events, Plasma widget, System Settings module, D-Bus API, CLI, Prometheus / waybar outputs.

The hardware is read **as Apple's macOS 26.5 driver reads it** (same reports, same schedule, same circuit breaker: `docs/PARITE-APPLE.md`). Only three named operations can ever write to the keyboard (see [Security](#security)).

## What works

Status legend: **hardware** = verified on the real A1314 ISO; **code** = covered by tests and a simulated keyboard, not observed on hardware; **not done** = planned or deliberately not implemented.

| Function | Status | Where |
|---|---|---|
| Battery % (kernel `power_supply`, = HID `0x47`), voltage `0x46`/`0x49` in mV | hardware | `akm-core/src/power.rs`, `read_policy.rs` |
| Charge estimate by chemistry (alkaline / NiMH / lithium), battery-change detection, time remaining | code (curves read from public datasheets, hypothesis) | `chemistry.rs`, `forecast.rs`, `batteries.rs` |
| "Apple display" percentage (IOBluetooth curve) | code (from disassembly) | `apple_model.rs` |
| Keyboard-driven alerts, GET Input `0x30` | hardware (`30 00` read without incident) | `passive.rs` |
| Firmware version `0x4F` vs embedded table, thresholds `0x60` | hardware (`0x0050` measured) | `firmware.rs`, `docs/FIRMWARE.md` |
| Name stored in the keyboard, read (`0x51`-`0x54`) | hardware | `devname.rs` |
| Name stored in the keyboard, write (`0x55`) | hardware: proven on the keyboard on 2026-10-02 (`akmctl rename --device-name <name>`, one confirmation, read back at once) | `docs/RENOMMER-CLAVIER.md` |
| Alias on this computer (BlueZ `Alias`) | hardware | `akmctl rename` |
| `WillShutdown` (`0x40`) at shutdown, as macOS | code (effect not observable) | `parity.rs`, `docs/PARITE-APPLE.md` |
| HID_CONTROL SUSPEND / EXIT_SUSPEND at sleep / wake, as macOS | code (fake `bluetoothd` tests) | `akm-hid-control`, `docs/VEILLE-HID.md` |
| Clean forget `0x41` then unpair (`akmctl repair`) | code (effect on the keyboard not measured) | `docs/RECONNEXION-PAIRAGE.md` §5.4 |
| Apple circuit breaker (3 silences: stop everything, BlueZ disconnect) | code (proptest, 512 sequences) | `apple_model.rs`, `read_policy.rs` |
| RSSI / TX power (BlueZ MGMT, relative dB, **not** a power level) | hardware (needs group `akm`) | `rssi-helper.c`, `signal.rs` |
| Link doctor (`akmctl doctor`), guided repair (`akmctl repair`) | hardware | `crates/akmctl/src/doctor.rs`, `repair.rs` |
| Tray icon (SNI + dbusmenu, dynamic battery icons) | hardware | `apple-kb-monitord/src/tray/` |
| KNotification events (14), PowerDevil dedupe, actions | hardware (`docs/AUDIT-INTEGRATION-KDE.md`) | `notify.rs`, `data/apple-kb-monitor.notifyrc` |
| Plasma widget (FR/EN) | hardware | `plasma/com.agenceapi.devicehub/` |
| System Settings module "Apple Keyboard" (5 pages) | hardware (captures in `kcm/captures/`) | `kcm/`, `docs/KCM.md` |
| Window `apihub-app` (tabs Keyboard, Keys, Diag), single instance | hardware + e2e under Xvfb | `apihub-app/src/` |
| Special keys table, manual mapping (udev hwdb) without keyd | hardware (`akmctl keys --check`) | `keymap.rs`, `docs/TOUCHES.md` |
| Fn mode and `hid_apple` parameters through polkit | hardware | `akm-helper` |
| LED control (evdev `EV_LED`; NumLock never switched on) | code | `led.rs` |
| BlueZ `BatteryProvider1` (fallback when the kernel has no node) | code | `bluez.rs` |
| Self-check every 15 min (`akmctl selftest`), local CI, e2e | hardware (runs on this machine) | `docs/QA-AUTOMATIQUE.md` |
| Other models (A1255, A1314 2009, Magic Keyboard 2015-2024) | **not tested** (in the table, [#23](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/23)) | `model.rs` |
| Several keyboards at once in the UI, Magic Keyboard battery report `0x90` | not done ([#119](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/119), [#95](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/95)) | — |

## Installation

Not on the AUR: build the package from this repository.

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
sudo usermod -aG akm "$USER"      # RSSI: rssi-helper is root:akm 0750; log in again
akmctl doctor                       # link, pairing, hidraw, BlueZ / UPower configuration
akmctl status
```

The package installs and **enables** everything itself (`apple-kb-monitor.install`): the user daemon `apple-kb-monitord.service`, the shutdown notice, the self-check timer, and the two system sleep / wake units. The udev rule `70-apple-kb-hidraw.rules` tags the hidraw node `uaccess`, so the active-seat user needs no group for the keyboard itself. keyd is **optional** and never touched (an example config is in `/usr/share/doc/apple-kb-monitor/examples/keyd/`). Details, upgrade notes and uninstall: [docs/INSTALL.md](docs/INSTALL.md).

## Daily use

```bash
akmctl status                  # battery, estimate, voltage, signal, Fn mode, firmware
akmctl status --json           # JSON schema 1 (scripts, widgets)
akmctl watch                   # one JSON line per change
akmctl history --since 7d      # battery history; history export --csv
akmctl graph --span 7d         # terminal chart
akmctl keys --check            # what each F-key does right now (kernel -> evdev -> KDE)
akmctl keymap kde-apply        # bind F4 (Launchpad) in KDE, never replacing a binding
akmctl set fnmode 2            # F-keys first (polkit dialog, all Apple keyboards)
akmctl rename "Desk keyboard"  # alias on this computer (nothing written to the keyboard)
akmctl firmware                # version read once per connection vs the embedded table
akmctl info                    # register map + cached values, never reads the keyboard
akmctl doctor                  # read-only link diagnosis; sudo adds the link-key check
akmctl repair                  # wake + reconnect; re-pair only after a typed confirmation
akmctl selftest                # the 15-minute health check, by hand
akmctl waybar | akmctl metrics # waybar JSON, Prometheus text
```

Everything but `dump`, `led` and the two write commands goes through the daemon over D-Bus: the CLI never opens the keyboard on its own. `akmctl --help`, `akmctl <cmd> --help` and `man akmctl` list every option. Exit codes: 0, 1 error, 2 daemon absent, 64 usage.

## KDE integration

- **System Settings → Input Devices → Keyboard → Apple Keyboard** (`kcm_applekeyboard`): pages State, Keys, Notifications, Name, Diagnostics. No write without a click. One write goes to the keyboard itself: the Name tab's "Write the name into the keyboard…" (one confirmation, then `akmctl rename --device-name=<name> --yes`, read back). Fn mode and parameters ask for administrator authentication (polkit). Apply rewrites only the `config.toml` keys you changed, every other byte kept, and never a file it could not read. [docs/KCM.md](docs/KCM.md)
- **Notifications**: the daemon is an application of System Settings → Notifications (`apple-kb-monitor.notifyrc`): popup, sound, history and Do Not Disturb per event; "Open" and "Repair…" buttons; one single reminder when PowerDevil already warns about the same keyboard. [docs/NOTIFICATIONS.md](docs/NOTIFICATIONS.md)
- **Tray**: icon drawn by the daemon (battery steps, charging, disconnected), tooltip with age of the last reading, menu with "Rename keyboard…", "Copy information", "Fn mode" (current `hid_apple.fnmode`, media keys first / F1–F12 first, through the daemon's `SetFnMode` and polkit), "Quit (hide icon)".
- **Plasma widget** `com.agenceapi.devicehub`: compact and full views, FR/EN, reads the daemon over the session bus; to place on a panel or the desktop. The notification-area icon is the daemon's (left click: window, right click: full menu).
- **Window** `apihub-app`: D-Bus activatable (`com.agenceapi.AppleKbMonitor`), single instance, launched from the icon, the widget or a notification button; no autostart. [docs/INTEGRATION-KDE.md](docs/INTEGRATION-KDE.md)
- **Special keys**: F1-F2 brightness, F3 Exposé, F7-F12 media and volume work through `hid_apple fnmode=1`; F4 and Eject are unbound in Plasma until `akmctl keymap kde-apply`. [docs/TOUCHES.md](docs/TOUCHES.md)

## Troubleshooting

Start with `akmctl doctor`, then `akmctl selftest`. Full table (symptom → cause → command): [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md).

| Symptom | First command |
|---|---|
| The keyboard does not reconnect | press a key; if nothing, switch it **off and on** (light at power-on), then `akmctl repair` |
| `Signal: n/a` | `id -nG \| grep akm` — add yourself to the group `akm`, log in again |
| Battery stuck or wrong | `akmctl status` shows the age of the reading; the keyboard's % only steps down at reconnections |
| No tray icon, no alerts | `systemctl --user status apple-kb-monitord.service` |
| F4 / Eject do nothing | `akmctl keys --check`, then `akmctl keymap kde-apply` |
| keyd crashed after an upgrade | never `keyd reload` (keyd 2.6.0 segfaults): `sudo systemctl restart keyd`, see [docs/KEYD.md](docs/KEYD.md) |

## Security

- **Daemon and clients are unprivileged.** The daemon is a user service hardened by systemd (`UMask=0077`, `MemoryMax`, `KeyringMode=private`…); it reaches the keyboard through the `uaccess` ACL of the active seat.
- **Six privileged helpers, each doing one thing** (`/usr/lib/apple-kb-monitor/`): `rssi-helper` (file capability `cap_net_admin`, `root:akm 0750`, RSSI only); `akm-helper` (`pkexec`, `hid_apple` parameters and `/etc/modprobe.d`); `akm-keymap-helper` (`pkexec`, validated udev hwdb file); `akm-hid-control` (root at sleep / wake through the system units, `pkexec` for the manual test; emits the byte `0x13` or `0x14` only, on the control socket of `bluetoothd`); `akm-hid-inspect` (`pkexec`, read-only: the L2CAP MTU of that socket, no verb, cannot send); `akm-doctor-fix` (`pkexec`, launched by `akmctl doctor --fix`: the BlueZ / UPower settings and the adapter autosuspend rule, with `--dry-run`). Polkit actions (`set-fnmode`, `install-keymap`, `hid-control`, `doctor-fix`) are `auth_admin` without `_keep`, local active sessions only, except the read-only `hid-inspect` (`allow_active = yes`), bound to its own executable (`polkit/com.agenceapi.AppleKbMonitor.policy`).
- **What is written to the keyboard**: only the **named operations of the register map** (`akm-core/src/registry.rs`, class `WriteApple`): `Shutdown` = Feature `0x40` (id alone, once at shutdown, default on, `[apple] will_shutdown`); `Forget` = Feature `0x41` (id alone, `akmctl repair` only, after a typed `OUBLIER`); `DeviceName` = Feature `0x55` (65 bytes, behind three locks, default refused). One function issues the write ioctl, with two fixed sizes (1 and 65 bytes), every byte logged, never retried; a test sweeps the 256 ids in the three directions and a source scan fails if a second write path appears. Reads are limited to `0x47`, `0x46`, `0x49`, GET Input `0x30`, and once per connection `0x4F`, `0x60`, `0x51`-`0x54`; `0x4C` (pairing record) and `0xFE` (froze the firmware twice) are never requested.
- **Data**: history and backups live under `~/.local/state/apple-kb-monitor/` (`0600`); nothing goes to the network (the firmware table is embedded); the sensitive bytes of `0x4C` are never published. Audits: [docs/AUDIT-SECURITE-2.md](docs/AUDIT-SECURITE-2.md), accepted keylogger trade-off of `uaccess`: [udev/README.md](udev/README.md).

## Known limits

- Only the A1314 ISO was tested on hardware; the 16 other models of the table rely on the kernel battery and are unverified ([#23](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/23)).
- The register map still carries **13 entries of unknown meaning** (11 Feature ids `0x45 0x4B 0xD0 0xD1 0xD4 0xD5 0xD8 0xF6 0xF7 0xFA 0xFB`, 2 Input ids `0x04 0x05`; `akmctl info --json`). Eleven ids refuse GET (`ERR_UNSUPPORTED_REQUEST`); their meaning comes from Apple's driver names only.
- The name stored in the keyboard is written by `akmctl rename --device-name <name>` (proven on the hardware); its persistence across a battery change is not measured ([#248](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/248)).
- The effect on the keyboard of `WillShutdown`, of `RecantConnection` and of the sleep / wake bytes is not observable from the host.
- BR/EDR RSSI is relative to the controller's golden receive range (0 = ideal), not a power level: the UI shows words, the JSON keeps `rssi_dbm` only as a deprecated mirror of `rssi_rel_db` ([#174](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/174)).
- The BCM2042 is an 8051-based Bluetooth 2.0 controller (Broadcom brief `2042-PB03-R`); whether its firmware images are signed is unknown; nothing here flashes anything.
- Two KNotification events (`FirmwareUpdate`, `BatteryReminder`) exist in Plasma but no daemon trigger calls them yet.
- The udev `uaccess` rule covers the 17 Bluetooth product ids only; wired Apple keyboards are no longer matched ([#155](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/155)).

## Architecture

```
            session bus (D-Bus)                           system bus
 ┌────────────────────────────────────────────┐    ┌──────────────────────────┐
 │ apple-kb-monitord  (user service, Rust)    │    │ bluetoothd (BlueZ)       │
 │  com.agenceapi.AppleKbMonitor1             │◄──►│  Device1, Adapter1,      │
 │   .Device  .Input  .Link  .Keymap  .Tray   │    │  BatteryProviderManager1 │
 │  tray SNI + dbusmenu, KNotification        │    └──────────────────────────┘
 │  akm-core: registry, read_policy, breaker, │              ▲
 │  apple_model, history, chemistry, keymap   │              │ pidfd_getfd, 1 byte
 └───────┬───────────────┬────────────────────┘    ┌─────────┴────────────────┐
         │ hidraw GET     │ child, cap_net_admin    │ akm-hid-control (root)   │
         ▼               ▼                          │ sleep / wake units       │
 /dev/hidrawN      rssi-helper (MGMT)              └──────────────────────────┘
 /sys/class/power_supply/hid-<mac>-battery*
         ▲
 clients ├─ akmctl (CLI)        ├─ apihub-app (egui window, D-Bus activatable)
         ├─ Plasma widget       ├─ kcm_applekeyboard (System Settings)
         └─ pkexec akm-helper (hid_apple params), akm-keymap-helper (udev hwdb)
            pkexec akm-doctor-fix (akmctl doctor --fix), akm-hid-inspect (read-only)
```

Repository layout and the full description: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

```
apihub-app/           Rust workspace: akm-core, apple-kb-monitord, crates/akmctl, crates/akm-helper
                      (akm-helper, akm-keymap-helper, akm-hid-control, akm-hid-inspect, akm-doctor-fix),
                      src/ (egui window), i18n/
rssi-helper.c         C helper with cap_net_admin (RSSI / TX power)
kcm/ plasma/ data/    System Settings module, Plasma widget, KNotification events
systemd/ dbus/ polkit/ udev/ sysusers/ modprobe/ keyd/ bluetooth/   system integration
scripts/ tests/       ci-local.sh, qa_checks.py; e2e (Xvfb + bubblewrap), fixtures, live (read-only)
docs/                 user docs, reverse-engineering notes, audits (index: docs/INDEX.md)
PKGBUILD, apple-kb-monitor.install   Arch Linux package
```

## Documentation

- Index of every document: [docs/INDEX.md](docs/INDEX.md)
- User: [INSTALL](docs/INSTALL.md), [CONFIGURATION](docs/CONFIGURATION.md), [FEATURES](docs/FEATURES.md), [TROUBLESHOOTING](docs/TROUBLESHOOTING.md), [TOUCHES](docs/TOUCHES.md) (keys, FR), [KCM](docs/KCM.md), [NOTIFICATIONS](docs/NOTIFICATIONS.md), [RECONNEXION-PAIRAGE](docs/RECONNEXION-PAIRAGE.md)
- Developer: [ARCHITECTURE](docs/ARCHITECTURE.md), [TESTING](docs/TESTING.md), [QA-AUTOMATIQUE](docs/QA-AUTOMATIQUE.md), [PARITE-APPLE](docs/PARITE-APPLE.md), [FIRMWARE](docs/FIRMWARE.md), [VEILLE-HID](docs/VEILLE-HID.md)
- Evidence: [HARDWARE-RAPPORTS-HID](docs/HARDWARE-RAPPORTS-HID.md), [CONTRE-AUDIT](docs/CONTRE-AUDIT.md), `docs/RE-*.md`, `docs/AUDIT-*.md`
- [CHANGELOG.md](CHANGELOG.md), [wiki (French)](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/wiki), [issues](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues), [milestones](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/milestones)

## License

[GPL-2.0-or-later](LICENSE). Author: Han — [AgenceAPI](https://gitea.pika.agenceapi.fr/adminapi).
