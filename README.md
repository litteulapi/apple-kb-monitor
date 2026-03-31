# apple-kb-monitor

Full telemetry monitor for Apple Wireless Keyboards on Linux, built by reverse-engineering the undocumented HID Feature Reports exposed by the Broadcom BCM2042/BCM20733 controllers.

**v2.0** — native BlueZ Battery Provider integration: your keyboard's precise battery percentage appears directly in KDE Plasma, GNOME, and any desktop environment that reads `org.bluez.Battery1`.

## What it reads

The Linux `hid-apple` kernel driver only exposes basic battery percentage via standard HID Battery Strength (Report 0x47). This tool reads **21 undocumented Feature Reports** to extract:

| Report | Data | Notes |
|--------|------|-------|
| `0x47` | Battery % (standard) | Rounded, same as `hid-apple` driver |
| `0xEA` | Battery % (precise) | Pre-rounding value from ADC |
| `0xF5` | Battery voltage (raw ADC) | 10-bit, 3.3V reference |
| `0xF4` | ADC calibration reference | |
| `0x5A` | Discharge curve (4 thresholds) | Millivolt thresholds for 100/75/50/25% |
| `0x5B` | Voltage reference pair | |
| `0x4F` | Firmware version | |
| `0xFF` | Build/revision number | |
| `0x51-53` | Device name string | 3 chunks from chip ROM |
| `0x4C` | Device identity (128-bit) | Internal key |
| `0x46` | BT connection interval + latency | Live values, renegotiated by controller |
| `0x49` | BT supervision timeout | |
| `0x4A` | Power management config | |
| `0x4B` | Device mode/class | |
| `0x09` | Device state flag | 1=OK, 0=LOW |
| `0xF6-F7` | Config registers | |
| MGMT | RSSI, TX power | Via BlueZ MGMT socket (requires `CAP_NET_ADMIN`) |
| D-Bus | Connection state, pairing | Via `org.bluez.Device1` |

## Desktop integration (v2.0)

In daemon mode, `apple-kb-monitor` registers as a **BlueZ Battery Provider** via the `BatteryProviderManager1` D-Bus API. This causes BlueZ to expose `org.bluez.Battery1` on the device path with the **precise** battery percentage from HID report `0xEA`.

The integration chain:

```
apple-kb-monitor daemon
    -> registers BatteryProvider1 on system D-Bus
        -> BlueZ creates Battery1 on /org/bluez/hci0/dev_XX_XX_XX_XX_XX_XX
            -> UPower discovers Battery1 (type: keyboard)
                -> KDE Plasma battery applet shows it
                -> KDE Bluedevil shows battery in BT panel
                -> GNOME power indicator shows it
                -> Any D-Bus consumer can read it
```

The daemon also monitors BlueZ `PropertiesChanged` signals for real-time device connect/disconnect detection.

## Supported keyboards

Tested:
- Apple Wireless Keyboard A1314 (aluminum, ISO) — BCM2042

Should work (same Broadcom HID register map):
- Apple Wireless Keyboard A1016 (white) — BCM2042
- Apple Wireless Keyboard A1255 (aluminum) — BCM2042
- Apple Magic Keyboard A1644 — BCM20733
- Apple Magic Keyboard A2449 (Touch ID) — BCM20733

## Install (Arch Linux / AUR)

```bash
# From AUR
yay -S apple-kb-monitor

# Or manually
git clone https://github.com/litteulapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

### Dependencies

- `python` (>= 3.10)
- `bluez` (>= 5.56 for Battery Provider API)
- `python-dbus-fast` (async D-Bus library)

Optional:
- `libnotify` — desktop notifications on low battery
- `bluez-utils` — `bluetoothctl` for BT management

## Setup

```bash
# Add user to input group (for non-root hidraw access)
sudo usermod -aG input $USER
# Log out and back in

# Enable the background monitor (systemd user service)
systemctl --user enable --now apple-kb-monitor.service
```

## Usage

```bash
# Quick battery + voltage check
apple-kb-monitor --once
# Output: Apple Wireless Keyboard (A1314, aluminum, ISO)   100% (fine:98%)  2.981V

# Full decoded device report
apple-kb-monitor --status

# Raw Feature Report dump (for RE work)
apple-kb-monitor --dump

# Live dashboard (auto-refresh)
apple-kb-monitor --watch

# JSON output (for scripts, widgets, Home Assistant, etc.)
apple-kb-monitor --json

# Waybar/polybar JSON output
apple-kb-monitor --waybar

# Battery/voltage history log
apple-kb-monitor --history

# RSSI (requires sudo for MGMT socket)
sudo apple-kb-monitor --status

# Daemon mode with Battery Provider (used by systemd service)
apple-kb-monitor --threshold 15 --interval 300

# Daemon without Battery Provider registration
apple-kb-monitor --no-provider
```

### Waybar integration

Add to your waybar config:

```json
"custom/apple-kb": {
    "exec": "apple-kb-monitor --waybar",
    "return-type": "json",
    "interval": 60,
    "format": "{}",
    "tooltip": true
}
```

## Permissions

| Feature | Required | How |
|---------|----------|-----|
| Battery, voltage, firmware, all HID reports | `input` group | `sudo usermod -aG input $USER` + re-login |
| RSSI, TX power | `CAP_NET_ADMIN` | Run with `sudo` |
| Desktop notifications | `libnotify` | `pacman -S libnotify` |
| Battery Provider (KDE/UPower) | system D-Bus | Automatic (no policy file needed) |

The udev rule (`99-apple-kb-hidraw.rules`) grants `input` group read/write access to Apple hidraw devices, including Bluetooth HID devices connected via uhid (matched by `DEVPATH` since uhid devices have no `idVendor` attribute).

## How it works

1. Discovers Apple Bluetooth keyboards via `/sys/class/hidraw/*/device/uevent`
2. Opens the `hidraw` device and sends `HIDIOCGFEATURE` ioctls for each known report ID
3. Decodes the binary responses based on our reverse-engineering of the BCM2042 register map
4. Queries BlueZ for RSSI via MGMT socket (`GET_CONN_INFO`, opcode `0x0031`) and connection properties via D-Bus
5. In daemon mode:
   - Registers as a BlueZ Battery Provider via `BatteryProviderManager1` D-Bus API
   - Monitors `PropertiesChanged` signals for device connect/disconnect events
   - Logs readings to `$XDG_RUNTIME_DIR/apple-kb-monitor/history.jsonl`
   - Sends desktop notifications via `notify-send` when battery drops below threshold

## Reverse engineering notes

The BCM2042 is a Broadcom single-chip Bluetooth HID controller with:
- ARM7TDMI core
- Bluetooth 2.0+EDR radio
- 10-bit ADC for battery voltage measurement
- Signed Apple firmware (not flashable from Linux)

The HID report descriptor only declares Reports 0x01 (keyboard), 0x47 (battery), 0x11-0x13 (consumer/vendor), and 0x09 (feature). Reports 0x46, 0x49-0x53, 0x5A-0x5B, 0x60, 0xEA-0xEB, 0xF4-0xF7, 0xFF are **undocumented** and were discovered by brute-force scanning all 256 Feature Report IDs.

The discharge calibration curve in Report 0x5A stores 4 voltage thresholds in millivolts that the firmware uses to map ADC readings to battery percentage. Typical values for 2xAA alkaline:

```
100% >= 2900 mV (2.900 V)
 75% >= 2450 mV (2.450 V)
 50% >= 2350 mV (2.350 V)
 25% >= 2000 mV (2.000 V)
```

### RSSI

RSSI is read via BlueZ MGMT `GET_CONN_INFO` (opcode `0x0031`), which triggers `HCI Read_RSSI` and `HCI Read_TX_Power` on the ACL connection handle. RSSI = 0 dBm is a valid measurement meaning optimal signal strength ("golden range"), not "unavailable". Stress-tested with 10 consecutive reads at 2s intervals with zero disconnects.

## License

GPL-2.0-or-later
