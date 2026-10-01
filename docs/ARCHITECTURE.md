# System architecture

> Line counts measured with `wc -l` on commit `c28fd8b`+ (2026-10-01): `apihub-app/src` 4 095 lines in 7 files (main 1032, keyboard 1065, bluez 576, tray 487, power 285, rssi 253, history 97); `rssi-helper.c` 132; `apple-kb-monitor` (Python) 2 693. Module headers in the source are the reference for design decisions.

## Overview

The repository is keyboard-only. The display/DDC/MQTT/Home Assistant part was removed (issue #59, tag `archive/avec-ecran`); it now lives in the private repository https://gitea.pika.agenceapi.fr/adminapi/lg-ddc-control. It has one Rust desktop binary (`apihub-app`), one tiny privileged C helper (`rssi-helper`), a legacy Python CLI and a set of system integration files.

```
+-----------------------------------------------------------+
|                 apihub-app (Rust, egui)                    |
|                                                           |
|  main.rs      UI (tabs Keyboard, Diag), polling thread,   |
|               shared state, battery graph, diagnostics    |
|  keyboard.rs  model table (17 PIDs), HID Feature Reports, |
|               wake monitor, LED state                     |
|  power.rs     kernel power_supply battery (source of      |
|               truth for the percentage)                   |
|  bluez.rs     BlueZ Battery Provider (zbus)               |
|  rssi.rs      runs rssi-helper (timeout, 10 s cache)      |
|  tray.rs      StatusNotifierItem + dbusmenu (zbus)        |
|  history.rs   JSONL battery history                       |
+-------------------------+---------------------------------+
                          | child process
                  rssi-helper (C, cap_net_admin+ep)
                  BlueZ MGMT GET_CONN_INFO (0x0031)
```

## Module responsibilities

- **main.rs** -- egui application and tray-first startup (window opens on "Show Window"). Two tabs: Keyboard (telemetry, history graph, time remaining) and Diag (checks: binaries, user service, `hidraw readable`, keyd config, udev rules, `hid_apple fnmode`, RSSI helper). A supervised polling thread reads the keyboard, the kernel battery, RSSI and feeds `Arc<Mutex<SharedState>>`; it is restarted after a panic and the poisoned lock is recovered.
- **keyboard.rs** -- model table `APPLE_MODELS` built from the kernel `hid-ids.h` (17 PIDs; vendors `05AC` and `004C`), `Family` (BCM2042 or Magic Keyboard). HID Feature Reports through `HIDIOCGFEATURE` on `/dev/hidrawN`, only for the BCM2042 family. Battery calibration curve (0x5A) validated before use. Wake monitor thread on Input Report 0x13, started even if the keyboard is absent at launch. CapsLock/NumLock state from sysfs.
- **power.rs** -- `kernel_battery(mac)`: resolves `/sys/class/power_supply/hid-<mac>-battery[-N]` through the HID parent (`HID_UNIQ`) with a fallback by name, strict MAC validation, `capacity` and `status` parsers separate from I/O and tested on a fake tree. No `unsafe`, no subprocess. The percentage shown comes from here; raw HID reports are diagnostics.
- **bluez.rs** -- exports one `org.bluez.BatteryProvider1` per connected keyboard under `/com/agenceapi/AppleKbMonitor/dev_AA_BB_CC_DD_EE_FF`, with a zbus `ObjectManager` on the root, and registers it with `org.bluez.BatteryProviderManager1`. No well-known bus name is requested. Adapter path resolved with `GetManagedObjects`. A small state machine re-registers when bluetoothd restarts or the adapter changes. UPower hides the BlueZ battery when the kernel already provides one with the same MAC, so the provider is a fallback.
- **rssi.rs** -- runs `/usr/lib/apple-kb-monitor/rssi-helper <MAC>` as a child with a 1.5 s timeout and a 10 s cache; value 127 means unavailable. The GUI never opens the MGMT socket itself (status `0x14` for unprivileged sockets). Override for tests: env `APPLE_KB_RSSI_HELPER`.
- **rssi-helper.c** -- opens the HCI control channel, sends `GET_CONN_INFO`, matches the reply to its request, prints `{"rssi":..,"tx_power":..,"max_tx_power":..}`. Exit codes 1 usage, 2 socket, 3 MGMT status, 4 timeout, 5 unavailable. The only binary with `cap_net_admin` (set by `post_install` with `setcap`, not in `package()` because fakeroot does not keep it).
- **tray.rs** -- pure zbus: `org.kde.StatusNotifierItem` and `com.canonical.dbusmenu` (info lines, "Show Window", "Quit"). Re-registers with the StatusNotifierWatcher whenever it reappears (`NameOwnerChanged`).
- **history.rs** -- JSONL store `~/.local/share/apple-kb-monitor/history.jsonl`, discharge rate and time remaining; invalid points (voltage <= 0, NaN) rejected.

## Data flow

```
Hardware / kernel
  Apple keyboard (BT HID) --> /dev/hidrawN                    HID Feature Reports, wake events
  hid-apple / hid-input   --> /sys/class/power_supply/hid-*   battery percentage and status
  BlueZ (D-Bus)           <-> org.bluez.Device1, Battery1     connection state, battery export
  BlueZ MGMT (HCI ctl)    <-- rssi-helper                     RSSI, TX power

System layer
  udev 70- uaccess        ACL on hidraw for the active seat user
  keyd                    F3-F6 to KDE shortcuts
  hid_apple               fnmode=1
  D-Bus policy            unprivileged sessions may call org.bluez

Desktop layer
  KDE / UPower            kernel battery (and BlueZ Battery1 when no kernel node)
  Plasma widget           apple-kb-monitor --json
  Bluedevil patch         battery %, firmware in the BT panel
```

## Privilege model

| Component | Privilege | Mechanism |
|---|---|---|
| hidraw read | user of the active seat | udev `uaccess` (`70-apple-kb-hidraw.rules`, must sort before `73-seat-late.rules`) |
| sysfs `power_supply` | any user | world-readable attributes |
| BlueZ D-Bus | unprivileged | system bus policy `send_destination="org.bluez"` |
| MGMT RSSI | `cap_net_admin` | file capability on `rssi-helper` only |

## File layout

```
/usr/bin/                         apihub-app, apple-kb-monitor
/usr/lib/apple-kb-monitor/        rssi-helper (cap_net_admin+ep)
/usr/lib/udev/rules.d/            70-apple-kb-hidraw.rules
/usr/lib/systemd/user/            apple-kb-monitor.service
/etc/keyd/                        apple-keyboard.conf (05ac:0256)
/etc/modprobe.d/                  hid_apple.conf (fnmode=1)
/etc/dbus-1/system.d/             com.agenceapi.AppleKbMonitor.conf
/usr/share/applications/          apihub-app.desktop
/usr/share/icons/hicolor/scalable/apps/   apihub-scarab.svg
/usr/share/plasma/plasmoids/      com.agenceapi.devicehub/
/usr/share/apple-kb-monitor/kde/  DeviceItem.qml (Bluedevil patch)
~/.local/share/apple-kb-monitor/  history.jsonl
```

## Review documents

`REVUE-ARCHITECTURE-GLOBALE.md`, `REVUE-ARCHITECTURE-CLAVIER.md` and `REVUE-CORRECTIFS.md` are dated audit snapshots (2026-10-01) and are not updated with the code. Target architecture steps: issues #60 to #62.
