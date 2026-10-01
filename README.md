<p align="center">
  <h1 align="center">apple-kb-monitor</h1>
  <p align="center">
    Full telemetry monitor for Apple Wireless Keyboards on Linux.<br>
    Reads the undeclared HID Feature Reports of BCM2042-based keyboards (A1255/A1314).
  </p>
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-2021-DEA584?logo=rust&logoColor=black" alt="Rust"></a>
  <a href="https://kernel.org/"><img src="https://img.shields.io/badge/Platform-Linux-FCC624?logo=linux&logoColor=black" alt="Linux"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--2.0--or--later-blue" alt="GPL-2.0-or-later"></a>
</p>

---

## Overview

The kernel exposes the battery percentage of these keyboards through the HID Battery Strength report (`0x47`, read as a Feature report by the `hid-input.c` battery quirk). On the A1314, 27 Feature report IDs answer GET_REPORT while the descriptor declares only `0x09` as Feature (and `0x47` as Input): the 25 others are undeclared. This tool decodes the ones whose meaning has been measured — battery voltage in mV, the firmware discharge table, firmware version, device name, paired host address — and keeps the others as raw bytes. See [docs/HARDWARE-RAPPORTS-HID.md](docs/HARDWARE-RAPPORTS-HID.md) and [docs/CONTRE-AUDIT.md](docs/CONTRE-AUDIT.md) for the evidence level of each claim.

- **`apihub-app`** — Rust/egui desktop app (2 tabs: Keyboard and Diag, plus a tray icon): keyboard telemetry, battery history, BlueZ battery provider
- **`rssi-helper`** — tiny C helper carrying `cap_net_admin`, the only privileged binary (RSSI / TX power)
- **`apple-kb-monitord`** — the daemon, single owner of the keyboard; **`akmctl`** — the CLI: `status`, `watch`, `history`, `graph`, `waybar`, `metrics`, `led`, `dump`, `doctor`, `repair`
- systemd user service, udev rule (`uaccess`), keyd config, Plasma widget, PKGBUILD, Gitea Actions CI

Documentation: [docs/](docs/) (FEATURES, CONFIGURATION, INSTALL, ARCHITECTURE, TROUBLESHOOTING, TESTING; reverse engineering: HARDWARE-RAPPORTS-HID, RE-*, CONTRE-AUDIT), [CHANGELOG.md](CHANGELOG.md) and the [wiki](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/wiki). Roadmap: [milestones](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/milestones).

## Features

| Feature | Source | Details |
|---|---|---|
| Battery % (displayed) | Kernel `power_supply` | `/sys/class/power_supply/hid-<mac>-battery*`, the node UPower reads (= Feature `0x47`); source of truth |
| Battery voltage | HID `0x46` (u16 LE, mV), cross-checked with `0xFF` bytes 1-2 (u16 BE) | Measured: moves with time and with a battery change. Rust daemon only; the legacy Python CLI still shows `0xF5` × 3.3/1023, which is a constant (see Known limits) |
| Filtered voltage | HID `0x49` (u16 LE, mV) | Smoothed value, meaning is a hypothesis |
| Discharge table | HID `0x5A` (= `0x60` = `0xEB`) | 4 × u16 BE in mV; mapping to 100/75/50/25 % is a hypothesis consistent with `0x47` |
| Firmware version | HID `0x4F` | u16 LE = `0x0050` = SDP/PnP `Version` (bcdDevice) |
| Device name | HID `0x51`-`0x54` | 4 × 8-byte ASCII chunks (Apple: `DeviceName1..4`) |
| Paired host | HID `0x4C` bytes 2-7 | Address of the paired host adapter, reversed; the 12 remaining bytes are sensitive and never published |
| RSSI / TX power | BlueZ MGMT via `rssi-helper` | `GET_CONN_INFO` (opcode `0x0031`), helper has `cap_net_admin+ep` |
| Connection state | D-Bus | `org.bluez.Device1` properties |

## HID Report Map (A1314 ISO, firmware `0x0050`)

| Report ID | Declared in descriptor | Meaning | Evidence |
|---|---|---|---|
| `0x47` | Input (Generic Device Controls `0x06` / Battery Strength `0x20`) | Battery % (read as Feature by the kernel quirk) | descriptor + kernel source + measured |
| `0x09` | Feature (vendor `FF01:0B`) | constant `01`; Apple name suggests a firmware Caps Lock delay flag | measured value, meaning hypothesis |
| `0x46` / `0xFF` | no | battery voltage, mV (LE / BE); `0xFF` byte 3 = `0x01` | measured |
| `0x49` | no | smoothed voltage, mV | measured value, meaning hypothesis |
| `0x5A` = `0x60` = `0xEB` | no | 4 voltages (mV) of the firmware discharge table | measured values, meaning hypothesis |
| `0x5B` | no | `0xF4` ‖ `0xF5` + 4 zero bytes | measured |
| `0xF4` / `0xF5` | no | constants 1740 / 900; `0xF5` is **not** a voltage (unchanged across a battery change) | measured; meaning unknown |
| `0x4F` | no | firmware version `0x0050` | measured |
| `0x51`-`0x54` | no | device name (4 × 8 bytes) | measured + Apple driver plist |
| `0x4C` | no | `0x03`, paired host address, 12 sensitive bytes | measured |
| `0xEA` | no | percentage-like value (98 when `0x47` = 99) | measured value, meaning unknown |
| `0x4A`, `0x4B`, `0x54`, `0x5C`, `0x5D`, `0xD1`, `0xD8`, `0xF6`, `0xF7`, `0xFE` | no | constants or zeros, meaning unknown; `0xFE` is never read (it preceded two link losses) | measured |

Reports were found by a read-only GET_REPORT scan of all 256 IDs (`HIDIOCGFEATURE`). Eleven more IDs (`0x40 0x41 0x44 0x45 0x50 0x55 0xD0 0xD4 0xD5 0xFA 0xFB`) exist but refuse GET (HANDSHAKE `ERR_UNSUPPORTED_REQUEST`); six of them are named in Apple's macOS driver (see [docs/RE-PILOTE-MACOS.md](docs/RE-PILOTE-MACOS.md)). Nothing is ever written to the keyboard.

## Hardware Compatibility

| Model | Controller | Status |
|---|---|---|
| Apple Wireless Keyboard A1314 (aluminum, ISO) | BCM2042 (identified on the A1255 by iFixit; not checked on an opened A1314) | **Tested** (fixtures in `tests/fixtures/a1314_iso/`) |
| A1255 (ANSI, ISO, JIS), A1314 2009 and aluminum (ANSI, ISO, JIS) | BCM2042 | In the model table, not tested |
| Magic Keyboard 2015 (A1644), with numeric keypad (A1843) | not verified | In the model table, not tested |
| Magic Keyboard 2021 (A2450, A2449, A2520) and 2024 (3 variants) | not verified | In the model table, not tested |

The 17 models come from `APPLE_MODELS` in `apihub-app/akm-core/src/model.rs` (PIDs from the kernel `hid-ids.h`; USB vendor `05AC` and Bluetooth vendor `004C` accepted). The raw undocumented HID reports are only read on the BCM2042 family; other models rely on the kernel battery.

## Installation

The package is not published on the AUR: build it from this repository.

### Manual

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

## Setup

No group membership is needed: the udev rule `70-apple-kb-hidraw.rules` tags Apple hidraw devices `uaccess`, so logind grants an ACL to the user of the active seat. Reconnect the keyboard (or `sudo udevadm trigger --subsystem-match=hidraw`) after installing.

Enable the daemon once (tray icon, low-battery notifications, history, Plasma widget data):

```bash
systemctl --user enable --now apple-kb-monitord.service
```

`apple-kb-monitord` is the single owner of the keyboard. The package does not enable it by itself: without this command it only starts on demand, when a client (`apihub-app`, the Plasma widget) calls `com.agenceapi.AppleKbMonitor1` on the session bus (D-Bus activation), so there is no tray icon nor low-battery alert until one of them has been opened.

The Python CLI (`apple-kb-monitor`, `apihub-settings`, `apple-kb-monitor.service`) was removed: Rust is the only language of the driver. Upgrading from it: see [docs/INSTALL.md](docs/INSTALL.md#upgrading-from-the-python-cli-before-310-6).

## Usage

```bash
akmctl status                  # battery, voltage, link, Fn mode
akmctl status --json           # JSON schema 1 (scripts, widgets, Home Assistant)
akmctl watch                   # one JSON line per change
akmctl history --since 7d      # battery history (UTC), --json available
akmctl history export --csv    # CSV on stdout
akmctl graph --span 7d         # terminal chart, 24h or 7d
akmctl waybar                  # JSON for a waybar custom module
akmctl metrics                 # Prometheus text format
akmctl led caps on             # LED (NumLock can only be switched off)
akmctl dump                    # the 3 safe reports only (0x47, 0x46, 0x49)
akmctl doctor                  # Bluetooth link diagnosis
```

Everything but `dump` and `led` reads the daemon over D-Bus: the CLI never talks to the keyboard on its own. `akmctl --help` and `man akmctl` list all options.

## Permissions

| Feature | Requirement | Setup |
|---|---|---|
| Battery, HID reports (hidraw) | `uaccess` ACL (active seat user) | Installed by the package (`70-apple-kb-hidraw.rules`) |
| RSSI, TX power | `cap_net_admin+ep` on `/usr/lib/apple-kb-monitor/rssi-helper` | Applied by the package `post_install` (`setcap`); retry by hand if it prints a warning |
| Desktop notifications | `libnotify` | `pacman -S libnotify` |

The rule matches Bluetooth HID devices connected via uhid (`KERNELS` `0005:05AC:*` and `0005:004C:*`) and wired Apple keyboards (`0003:05AC:*`). It must sort before `73-seat-late.rules`, hence the `70-` prefix.

## Dependencies

Everything is compiled Rust (`apihub-app`, `apple-kb-monitord`, `akmctl`); no interpreter is needed at runtime (see [docs/INSTALL.md](docs/INSTALL.md)).

| Dependency | Type | Purpose |
|---|---|---|
| `bluez` | Runtime | Bluetooth stack, Battery Provider API |
| `keyd` | Runtime | Apple special keys remapping |
| `bluez-utils` | Optional | `bluetoothctl` for manual pairing |
| `libnotify` | Optional | `notify-send` for low-battery notifications |
| `rust`, `gcc` | Build | `apihub-app`, `rssi-helper` |

## How It Works

1. Discovers Apple Bluetooth keyboards via `/sys/class/hidraw/*/device/uevent`
2. Reads the battery percentage from the kernel `power_supply` node of the keyboard (matched by MAC)
3. The daemon opens the `hidraw` device and sends `HIDIOCGFEATURE` for `0x47`, `0x46` and `0x49` only (BCM2042 family), only when a key was pressed in the last minute, under a cross-process lock (`akm_core::read_policy`)
4. Runs `rssi-helper` for RSSI (BlueZ MGMT `GET_CONN_INFO`, opcode `0x0031`) and reads connection properties from BlueZ over D-Bus; exports the battery to BlueZ as `org.bluez.Battery1`
5. The daemon logs readings to `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl` (default `~/.local/state/...`, the single history file) and sends desktop notifications when the battery drops below a threshold

## Reverse Engineering Notes

The BCM2042 is a Broadcom single-chip Bluetooth HID controller. Broadcom product brief `2042-PB03-R`:

- on-board **8051** processor, 108 KB ROM, 22 KB RAM, 20 KB boot ROM, ROM-based design
- **Bluetooth 2.0** (the brief does not mention EDR; a third-party BM2042 module sheet says "2.0+EDR compatible", and a public `hcitool info` of an Apple Wireless Keyboard shows no EDR feature bit)
- no public register map; no ADC resolution is published

Firmware: the only known update channel for this generation is Apple's legacy macOS updater over Bluetooth. Whether that image is signed is **not known** (the "no signature" result of K. Chen, Black Hat 2009, concerns the wired USB keyboard with a Cypress MCU). Nothing in this project writes to the keyboard.

The HID report descriptor (224 bytes) declares Reports `0x01` (keyboard, LEDs), `0x47` (battery, Input), `0x11`-`0x13` (Eject/Fn, media, vendor bits) and `0x09` (the only Feature). The undeclared Feature reports are listed above.

### Discharge table

Report `0x5A` (identical copies `0x60` and `0xEB`) holds 4 voltages, big-endian, in mV. Measured on the test unit: 2954 / 2506 / 2404 / 2054 mV. The association with 100/75/50/25 % is a hypothesis (the code default 2900/2450/2350/2000 has no source).

### RSSI

RSSI is read via BlueZ MGMT `GET_CONN_INFO` (opcode `0x0031`) by the `rssi-helper` binary, which triggers `HCI Read_RSSI` and `HCI Read_TX_Power`. On a BR/EDR link, Read_RSSI is **not** a power in dBm: it is the gap in dB to the controller's Golden Receive Power Range (Core Spec Vol 4 Part E §7.5.4); 0 means "inside the ideal range". The Rust daemon exposes it as a relative value (#174); the legacy Python CLI still labels it dBm.

## Project Structure

```
apihub-app/                 # Rust workspace: akm-core, apple-kb-monitord, crates/akmctl (CLI), crates/akm-helper, egui GUI (src/)
rssi-helper.c               # C helper with cap_net_admin (RSSI / TX power)
udev/ systemd/ keyd/ modprobe/ dbus/             # system integration
plasma/ kde/                # Plasma widget, Bluedevil panel patch
tests/                      # fixtures/ (real A1314 capture), live/ (reverse-engineering tools + check_keyboard.sh)
.gitea/workflows/ci.yml     # CI
docs/                       # documentation
PKGBUILD, .SRCINFO, apple-kb-monitor.install   # Arch Linux package
```

## Known limits

- The legacy Python CLI was removed (3.1.0-6): `akmctl` replaces it and no longer publishes the sensitive bytes of `0x4C` ([#200](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/200), [#201](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/201)).
- Only the A1314 ISO has been tested on hardware.

## Acknowledgments

- BCM2042 HID Feature Report reverse engineering (read-only)
- Linux HID subsystem (`hidraw`, `HIDIOCGFEATURE`)
- BlueZ MGMT interface
- Arch Linux packaging ecosystem

## License

[GPL-2.0-or-later](LICENSE)

## Author

Han — [AgenceAPI](https://gitea.pika.agenceapi.fr/adminapi)
