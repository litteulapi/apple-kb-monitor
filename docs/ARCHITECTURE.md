# System architecture

## Overview

apple-kb-monitor is structured around two Rust binaries and a set of system integration files. The primary binary (`apihub-app`) is a monolithic desktop application that consolidates keyboard telemetry, monitor control, MQTT bridging, and brightness handling into a single process.

```
+------------------------------------------------------------------+
|                     apihub-app (Rust, egui)                       |
|                     ~3900 LOC, 8 modules                         |
|                                                                  |
|  +------------------+  +------------------+  +----------------+  |
|  | main.rs (2126)   |  | ddc.rs (355)     |  | keyboard.rs    |  |
|  |                  |  |                  |  | (401)          |  |
|  | egui UI engine   |  | DDC/CI I2C       |  | HID Feature    |  |
|  | 6-tab layout     |  | driver           |  | Reports        |  |
|  | System tray      |  |                  |  | 21 registers   |  |
|  | App presets      |  | I2C_RDWR ioctl   |  | Wake monitor   |  |
|  | Battery graph    |  | Bus auto-detect  |  | LED state      |  |
|  | Profile persist  |  | 34 polled VCPs   |  | 10 KB models   |  |
|  | Background poll  |  | 0x02 smart poll  |  | HIDIOCGFEATURE |  |
|  +------------------+  +------------------+  +----------------+  |
|                                                                  |
|  +------------------+  +------------------+  +----------------+  |
|  | bluez.rs (230)   |  | brightness.rs    |  | mqtt.rs (339)  |  |
|  |                  |  | (210)            |  |                |  |
|  | Battery Provider |  | F1/F2 evdev      |  | In-process     |  |
|  | zbus 4 blocking  |  | handler          |  | MQTT client    |  |
|  | Battery1 iface   |  |                  |  | (rumqttc)      |  |
|  | Auto-register    |  | Raw input_event  |  |                |  |
|  | with BlueZ       |  | DDC write        |  | 15 HA entities |  |
|  |                  |  | KDE OSD          |  | Bidir controls |  |
|  |                  |  | Circadian curve  |  | Select + Num   |  |
|  +------------------+  +------------------+  +----------------+  |
|                                                                  |
|  +------------------+  +------------------+                      |
|  | rssi.rs (137)    |  | history.rs (90)  |                      |
|  |                  |  |                  |                      |
|  | BlueZ MGMT API   |  | JSONL append-    |                      |
|  | AF_BLUETOOTH     |  | only store       |                      |
|  | socket           |  |                  |                      |
|  | Opcode 0x0031    |  | Discharge rate   |                      |
|  | RSSI + TX power  |  | Time remaining   |                      |
|  +------------------+  +------------------+                      |
+------------------------------------------------------------------+
```

## Module responsibilities

### main.rs (2126 LOC)

The application entry point and UI engine. Responsibilities:

- **egui application loop** -- 6 tabs (Keyboard, Display, Advanced, System, MQTT, Diagnostics)
- **System tray** -- ksni-based KDE StatusNotifierItem with scarab icon, battery tooltip, and quit action
- **App Presets** -- automatic picture mode switching based on active window class via KWin D-Bus scripting (Wayland-native, replaces xdotool)
- **Battery history graph** -- painter-based 24h dual-axis chart (battery % + voltage)
- **Factory Reset buttons** -- VCP 0x04 (full reset), 0x05 (brightness/contrast), 0x08 (color) with confirmation dialog
- **PBP mirror registers** -- displays sub-display settings when split mode is active
- **Background polling threads** -- spawns dedicated threads for DDC reads, keyboard reads, MQTT, and wake event monitor
- **Shared state** -- `Arc<Mutex<SharedState>>` for thread-safe data exchange between pollers and UI
- **Profile persistence** -- save/restore full DDC state as named profiles to `~/.config/apple-kb-monitor/profiles.json`
- **Config file I/O** -- reads `~/.config/apple-kb-monitor/config.toml` (fallback: `/etc/apple-kb-monitor/config.toml`)
- **Diagnostics** -- 15 system checks (binary presence, service status, hardware access, config files, permissions)

### ddc.rs (355 LOC)

Direct I2C DDC/CI driver. No subprocess, no ddcutil.

- **4-tier polling** -- HOT (every cycle: brightness, volume, backlight, 0x02), WARM (every 4th: RGB gains, black levels, sharpness), COLD (every 30th: picture mode, input, info VCPs), PBP (conditional on split mode)
- **VCP 0x02 smart polling** -- reads New Control Value flag; skips WARM tier when the monitor reports no OSD changes
- **34 polled VCPs** across tiers, **22 writable** via slider controls
- **Video Black Level RGB** -- newly discovered writable VCPs 0x6C/0x6E/0x70
- **Bus auto-detection** -- probes all `/dev/i2c-*` for a DDC-capable display by reading VCP 0xDF (version)
- **Read** -- `I2C_RDWR` ioctl with 2-message transaction (write request + read reply), validates response opcode, VCP code, result code, length, and checksum
- **Write** -- `I2C_SLAVE` + `libc::write` (required for NVIDIA I2C adapters that reject `I2C_RDWR` for writes)
- **Bus lock** -- `Mutex<()>` serializes all I2C transactions to prevent bus contention
- **NVIDIA workaround** -- double-read (flush + real read) to handle pipeline aliasing where rapid sequential reads return the previous request's data

### keyboard.rs (401 LOC)

Pure Rust HID Feature Report reader for BCM2042/BCM20733 keyboards.

- **10 keyboard models** -- full PID table for all known Apple Wireless/Magic keyboards (6 BCM2042, 4 BCM20733)
- **21 HID Feature Reports** decoded via `HIDIOCGFEATURE` ioctl on `/dev/hidrawN`
- **Structured output** -- `KbReport` struct with typed fields for battery (3 methods), voltage, firmware, calibration curve, identity, BT parameters
- **ADC calibration** -- decodes the 4-point discharge curve (0x5A) for voltage-to-percentage interpolation
- **Battery analysis** -- type detection (alkaline, NiMH, lithium) from voltage range and calibration shape
- **Wake event monitor** -- dedicated thread reads HID Input Report 0x13 (vendor FF01 usage page) for connection/wake events
- **LED state reader** -- reads CapsLock and NumLock state from sysfs for badge display in UI

### bluez.rs (230 LOC)

BlueZ Battery Provider via D-Bus.

- **zbus 4 blocking API** on a dedicated thread
- **BatteryProvider1** interface registered with BlueZ at `/com/agenceapi/AppleKbMonitor`
- **Battery1 object** exported per device -- BlueZ picks this up and creates the standard `org.bluez.Battery1` interface that UPower, KDE Plasma, and GNOME read natively
- **Atomic updates** -- `AtomicU8` for lock-free battery percentage updates from the polling thread

### brightness.rs (210 LOC)

F1/F2 brightness handler via raw evdev.

- **keyd virtual keyboard** -- opens the keyd evdev device and listens for `KEY_BRIGHTNESSDOWN` (224) and `KEY_BRIGHTNESSUP` (225) events
- **DDC write** -- adjusts monitor brightness via the `ddc` module in 5% steps
- **KDE OSD** -- sends brightness notification via `qdbus6` to the Plasma OSD
- **Circadian curve** -- `circadian_brightness()` returns a target brightness based on time of day (30% at night, ramp to 70% by 9 AM, hold, ramp down to 30% by 9 PM)
- **Cached state** -- `AtomicI32` avoids redundant DDC reads on repeated key presses

### mqtt.rs (339 LOC)

In-process MQTT client for Home Assistant.

- **rumqttc** async client running on a dedicated thread
- **15 HA auto-discovery entities**:
  - Keyboard: battery (%), voltage (mV), RSSI (dBm), TX power (dBm) sensors + connected binary_sensor
  - Monitor: brightness, contrast, volume, color temp, usage hours, backlight PWM sensors
  - Monitor controls: brightness number, volume number, picture mode select (14 modes), input source select (DP/HDMI1/HDMI2)
- **Bidirectional** -- subscribes to brightness, volume, picture mode, and input source command topics; writes DDC values on incoming messages
- **Connection state** -- `Arc<Mutex<bool>>` for UI status display
- **Config-driven** -- broker, port, auth, topic prefix, monitor model all from config.toml

### rssi.rs (137 LOC)

Bluetooth RSSI and TX power reader via the BlueZ MGMT API.

- **Pure Rust** -- replaces the C `rssi-helper` binary
- **AF_BLUETOOTH socket** with `BTPROTO_HCI`, `HCI_CHANNEL_CONTROL`
- **MGMT opcode 0x0031** (Get Connection Info) -- parses RSSI (dBm) and TX power (dBm) from the response
- **Requires** `CAP_NET_ADMIN` or root -- returns `None` on privilege failure (non-fatal)

### history.rs (90 LOC)

Battery history logging and analytics.

- **JSONL append-only store** at `~/.local/share/apple-kb-monitor/history.jsonl`
- **Per-reading entries** -- timestamp, battery percentage, voltage
- **Discharge rate estimation** -- calculates %/hour from recent history window
- **Time remaining prediction** -- extrapolates battery-empty time from current discharge rate
- **Best-effort** -- silently ignores write errors (history is non-critical)

## ddc-tool (standalone CLI, 300 LOC)

Separate Rust binary for DDC/CI operations from the command line. Uses the same I2C protocol as `ddc.rs` but is a standalone tool without GUI dependencies.

```
ddc-tool
  read <bus> <vcp|all>    I2C_RDWR read, 100ms per VCP
  write <bus> <vcp> <val> I2C_SLAVE + write(), 30ms
  json <bus>              30 essential VCPs, JSON output, ~5s
```

85 VCP codes mapped (20 standard MCCS + 10 LG vendor + 10 newly decoded + 16 mirror + 11 unknown + 5 VGA legacy + 3 black level + 10 monitor info).

## Data flow

```
Hardware Layer
  Apple Keyboard (BT HID)  -----> /dev/hidrawN     (Feature Reports)
  Apple Keyboard (BT)      -----> BlueZ (D-Bus)    (connection state)
  Apple Keyboard (BT)      -----> AF_BLUETOOTH      (MGMT RSSI)
  LG Monitor (I2C)         <----> /dev/i2c-N        (DDC/CI @ 0x37)
  keyd virtual keyboard    -----> /dev/input/eventN  (F1/F2 events)

Application Layer (apihub-app, single process)
  keyboard.rs    reads 21 HID Feature Reports, wake events (0x13), LED state
  rssi.rs        reads RSSI/TX via AF_BLUETOOTH MGMT socket
  ddc.rs         reads 34 VCPs (4-tier smart polling), writes 22 VCPs
  brightness.rs  listens for evdev F1/F2, writes DDC, sends KDE OSD
  bluez.rs       exports Battery1 interface via zbus D-Bus
  mqtt.rs        publishes 15 HA entities, bidirectional controls
  history.rs     battery history, discharge rate, time remaining
  main.rs        6-tab egui UI, system tray (ksni), app presets, battery graph

System Layer
  keyd             remaps F3-F6 to KDE shortcuts (Meta+Z/G/L/D)
  hid_apple        fnmode=1 (media keys as default Fn row)
  udev rules       hidraw group permissions for input group
  D-Bus policy     allows Battery Provider registration with BlueZ

Desktop Layer
  KDE Battery      reads Battery1 from BlueZ (populated by bluez.rs)
  Plasma widget    reads apple-kb-monitor --json + ddc-tool json
  Bluedevil patch  shows battery %, firmware, profiles in BT panel
```

## Threading model

```
Thread 1: egui UI loop (main thread)
  - Reads SharedState, renders 6 tabs
  - Handles user input (sliders, buttons, profile save/load)
  - Sends DDC write commands
  - Renders battery history graph (painter-based)

Thread 2: DDC poller
  - 4-tier smart polling: HOT every cycle, WARM every 4th (skipped if 0x02=0),
    COLD every 30th, PBP conditional on split mode
  - Updates SharedState.ddc

Thread 3: Keyboard poller
  - Reads 21 HID Feature Reports every 5s
  - Reads LED state from sysfs
  - Updates SharedState.keyboard

Thread 4: BlueZ Battery Provider
  - zbus blocking connection loop
  - Reads AtomicU8 for current battery %

Thread 5: Brightness handler
  - Blocks on evdev read() for F1/F2 key events
  - Writes DDC brightness + sends KDE OSD

Thread 6: MQTT client (optional, started from MQTT tab)
  - rumqttc event loop
  - Publishes 15 HA entities, processes bidirectional commands

Thread 7: Wake event monitor
  - Blocks on hidraw read() for Input Report 0x13
  - Updates last-wake timestamp for UI display

Thread 8: System tray (ksni)
  - StatusNotifierItem D-Bus service
  - Battery tooltip updated from SharedState, quit action
```

## File layout

```
/usr/bin/
  apihub-app                    Rust GUI (primary interface)
  ddc-tool                      Rust DDC/CI CLI
  apple-kb-monitor              Python CLI daemon (legacy)

/etc/
  apple-kb-monitor/
    config.toml.example         Configuration template
  keyd/
    apple-keyboard.conf         13 Apple special keys (05ac:0256)
  modprobe.d/
    hid_apple.conf              fnmode=1
  dbus-1/system.d/
    com.agenceapi.AppleKbMonitor.conf   D-Bus policy

/usr/lib/
  systemd/user/
    apple-kb-monitor.service    Legacy CLI daemon service
  udev/rules.d/
    99-apple-kb-hidraw.rules    hidraw group permissions

/usr/share/
  applications/
    apihub-app.desktop          KDE app launcher entry
  icons/hicolor/scalable/apps/
    apihub-scarab.svg           Application icon
  plasma/plasmoids/
    com.agenceapi.devicehub/    KDE Plasma widget
  apple-kb-monitor/kde/
    DeviceItem.qml              Bluedevil panel patch

~/.config/apple-kb-monitor/
  config.toml                   User configuration (copied from example)
  profiles.json                 Saved DDC profiles

~/.local/share/apple-kb-monitor/
  history.jsonl                 Battery history log
```
