# System architecture

## Overview

```
+------------------------------------------------------------------+
|                        User interfaces                           |
|                                                                  |
|  CLI (12 modes)    Waybar    Plasma Widget    apihub-settings    |
|  --once/--status   --waybar  (ApiHub)         (PySide6)          |
|  --json/--dump                                                   |
|  --watch/--graph                                                 |
|  --export-csv                                                    |
|  --metrics                                                       |
|  --history                                                       |
|  --led                                                           |
|  --auto-brightness                                               |
+-------+-----------+---------------+---------------+--------------+
        |           |               |               |
        v           v               v               v
+------------------------------------------------------------------+
|                    apple-kb-monitor daemon                        |
|                    (Python, async, dbus-fast)                     |
|                                                                  |
|  +------------------+  +------------------+  +----------------+  |
|  | HID Feature      |  | BlueZ D-Bus      |  | History        |  |
|  | Reports          |  | Integration      |  | & Analytics    |  |
|  |                  |  |                  |  |                |  |
|  | 21 registers     |  | Battery Provider |  | JSONL logging  |  |
|  | via hidraw       |  | (Battery1 iface) |  | Discharge rate |  |
|  | ioctl            |  |                  |  | Remaining time |  |
|  |                  |  | PropertiesChanged|  | Battery type   |  |
|  | HIDIOCSFEATURE   |  | InterfacesAdded  |  | detection      |  |
|  | HIDIOCGFEATURE   |  | signals          |  |                |  |
|  +------------------+  +------------------+  +----------------+  |
|                                                                  |
|  +------------------+  +------------------+  +----------------+  |
|  | HID Input        |  | RSSI             |  | MQTT           |  |
|  | Report 0x13      |  |                  |  |                |  |
|  |                  |  | rssi-helper      |  | Home Assistant |  |
|  | HidrawMonitor    |  | (C binary)       |  | auto-discovery |  |
|  | async fd watch   |  | CAP_NET_ADMIN    |  |                |  |
|  | wake/connection  |  | BlueZ MGMT API   |  | battery,       |  |
|  | events           |  | opcode 0x0031    |  | voltage, rssi  |  |
|  +------------------+  +------------------+  +----------------+  |
|                                                                  |
|  +------------------+  +------------------+                      |
|  | SDP Records      |  | LED Control      |                      |
|  |                  |  |                  |                      |
|  | GetServiceRecords|  | HIDIOCSFEATURE   |                      |
|  | Profile parsing  |  | Caps Lock        |                      |
|  | HID, PnP, SPP   |  | Num Lock         |                      |
|  +------------------+  +------------------+                      |
+------------------------------------------------------------------+
        |                       |                       |
        v                       v                       v
+------------------+  +------------------+  +------------------+
| /dev/hidrawN     |  | D-Bus            |  | notify-send      |
| (kernel hid)     |  | (system bus)     |  | (desktop notif)  |
+------------------+  +------------------+  +------------------+
```

## DDC/CI brightness pipeline

```
+-------------------+    +--------------------+    +------------------+
| Physical keyboard |    | apple-brightness   |    | LG 34GN850       |
| F1 / F2 key       |--->| -daemon            |--->| monitor          |
| (evdev event)     |    |                    |    |                  |
+-------------------+    | 1. Read current    |    | I2C bus (i2c-6)  |
                         |    brightness      |    | DDC addr 0x37    |
                         | 2. +/- 1%          |    | VCP 0x10         |
                         | 3. ddc-tool write   |    |                  |
                         | 4. KDE OSD via     |    +------------------+
                         |    D-Bus            |
                         +--------------------+
                                  |
                                  v
                         +--------------------+
                         | KDE Plasma OSD     |
                         | brightnessChanged  |
                         | D-Bus signal       |
                         +--------------------+
```

## ddc-tool architecture

```
+-------------------+
| ddc-tool (Rust)   |
|                   |
| Commands:         |
|   read <bus> <vcp>|-----> I2C_RDWR ioctl (read)   ---> 100ms
|   read <bus> all  |-----> 85 VCPs sequential       ---> ~10s
|   write <bus> ... |-----> I2C_SLAVE + write()      ---> 30ms
|   json <bus>      |-----> 30 essential VCPs        ---> ~5s
+-------------------+
         |
         v
+-------------------+
| /dev/i2c-N        |
| DDC/CI @ 0x37     |
|                   |
| Protocol:         |
| Write: [0x51, len,|
|   0x03, vcp,      |
|   val_hi, val_lo, |
|   checksum]       |
|                   |
| Read:  [0x6E, len,|
|   0x02, result,   |
|   vcp, type,      |
|   max_hi, max_lo, |
|   cur_hi, cur_lo, |
|   checksum]       |
+-------------------+
```

## keyd key mapping

```
+---------------------------+     +-------------------+
| Apple Wireless Keyboard   |     | keyd daemon       |
| USB ID: 05ac:0256         |---->| (system service)  |
|                           |     |                   |
| F1  (brightnessdown)      |     | F3 -> Meta+Z      |
| F2  (brightnessup)        |     | F4 -> Meta+G      |
| F3  (scale / expose)      |     | F5 -> Meta+L      |
| F4  (dashboard)           |     | F6 -> Meta+D      |
| F5  (kbdillumdown)        |     | F1,F2 passthrough  |
| F6  (numlock/kbdillumup)  |     | F7-F12 native      |
| F7  (previoussong)        |     +-------------------+
| F8  (playpause)           |              |
| F9  (nextsong)            |              v
| F10 (mute)                |     +-------------------+
| F11 (volumedown)          |     | KDE Plasma /      |
| F12 (volumeup)            |     | Wayland           |
| Eject                     |     | compositor        |
+---------------------------+     +-------------------+
```

## KDE integration layers

```
+-------------------------------------------------------------+
|                     KDE Plasma Desktop                       |
|                                                              |
|  +------------------------+  +---------------------------+   |
|  | Bluedevil Panel        |  | ApiHub Widget             |   |
|  | (patched DeviceItem)   |  | (com.agenceapi.devicehub) |   |
|  |                        |  |                           |   |
|  | - Battery %            |  | CompactRepresentation     |   |
|  | - Firmware version     |  |   (panel icon + badge)    |   |
|  | - BT profiles          |  |                           |   |
|  | - Device class         |  | FullRepresentation        |   |
|  |                        |  |   (popup with tabs)       |   |
|  +------------------------+  |   - Keyboard telemetry    |   |
|             |                |   - Monitor DDC controls   |   |
|             v                |   - Brightness slider      |   |
|  +------------------------+  |   - Volume slider          |   |
|  | BlueZ Battery Provider |  |   - Picture mode           |   |
|  | (Battery1 interface)   |  +---------------------------+   |
|  | from apple-kb-monitor  |               |                  |
|  +------------------------+               v                  |
|                              +---------------------------+   |
|                              | apple-kb-monitor --json   |   |
|                              | ddc-tool json 6           |   |
|                              +---------------------------+   |
+-------------------------------------------------------------+

+-------------------------------------------------------------+
|                  apihub-settings (PySide6)                   |
|                                                              |
|  +------------------------+  +---------------------------+   |
|  | System Tray Icon       |  | Main Window               |   |
|  | (scarab icon)          |  |                           |   |
|  +------------------------+  | Tab: Monitor              |   |
|             |                |   - Brightness slider      |   |
|             v                |   - Contrast slider        |   |
|  +------------------------+  |   - Volume slider          |   |
|  | QThread data loader    |  |   - Input source           |   |
|  | (background I2C reads) |  |                           |   |
|  | with threading.Lock    |  | Tab: Keyboard              |   |
|  +------------------------+  |   - Battery %              |   |
|             |                |   - Voltage                |   |
|             v                |   - RSSI                   |   |
|  +------------------------+  |   - Firmware               |   |
|  | ddc-tool (subprocess)  |  +---------------------------+   |
|  | apple-kb-monitor --json|                                  |
|  +------------------------+                                  |
+-------------------------------------------------------------+
```

## Data flow summary

```
Hardware Layer:
  Apple Keyboard (BT) -----> /dev/hidrawN (HID reports)
  Apple Keyboard (BT) -----> BlueZ (D-Bus)
  LG Monitor (I2C) --------> /dev/i2c-6 (DDC/CI @ 0x37)

Daemon Layer:
  apple-kb-monitor ---------> hidraw ioctl (21 Feature Reports)
  apple-kb-monitor ---------> HidrawMonitor (Input Report 0x13)
  apple-kb-monitor ---------> rssi-helper (BlueZ MGMT subprocess)
  apple-kb-monitor ---------> D-Bus Battery Provider (Battery1)
  apple-kb-monitor ---------> D-Bus signals (connect/disconnect)
  apple-kb-monitor ---------> SDP service records
  apple-kb-monitor ---------> JSONL history file
  apple-kb-monitor ---------> MQTT broker (Home Assistant)
  apple-kb-monitor ---------> notify-send (low battery)
  apple-brightness-daemon ---> ddc-tool write (F1/F2)
  apple-brightness-daemon ---> D-Bus OSD signal

System Layer:
  keyd --------------------->(remaps F3-F6 to KDE shortcuts)
  hid_apple (fnmode=1) -----> kernel (media keys as default)
  udev rules --------------->(hidraw group permissions)

Desktop Layer:
  KDE Battery Provider -----> Bluedevil panel (patched)
  Plasma widget (ApiHub) ---> apple-kb-monitor --json
  Plasma widget (ApiHub) ---> ddc-tool json
  apihub-settings ----------> ddc-tool (subprocess)
  apihub-settings ----------> apple-kb-monitor --json
```

## File layout

```
/usr/bin/
  apple-kb-monitor              Main daemon + CLI
  apple-brightness-daemon       F1/F2 brightness handler
  apple-brightness-down         Brightness -1% helper
  apple-brightness-up           Brightness +1% helper
  ddc-tool                      Rust DDC/CI binary (manual install)

/usr/lib/
  apple-kb-monitor/
    rssi-helper                 C binary (CAP_NET_ADMIN)
  systemd/user/
    apple-kb-monitor.service    Battery provider daemon
    apple-brightness.service    Brightness daemon
  udev/rules.d/
    99-apple-kb-hidraw.rules    hidraw permissions

/etc/
  keyd/
    apple-keyboard.conf         13 special keys
  modprobe.d/
    hid_apple.conf              fnmode=1

/usr/share/
  apple-kb-monitor/
    kde/DeviceItem.qml          Bluedevil patch

~/.local/share/
  plasma/plasmoids/
    com.agenceapi.devicehub/    Plasma widget (user install)
```
