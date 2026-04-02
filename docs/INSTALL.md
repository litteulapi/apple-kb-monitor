# Installation guide

## Prerequisites

- Arch Linux or Manjaro (PKGBUILD provided)
- Bluetooth adapter supported by BlueZ
- Apple Wireless Keyboard paired via `bluetoothctl`
- For ddc-tool: I2C bus accessible (`i2c-dev` module loaded)
- For Plasma widget: KDE Plasma >= 6.0

## Package installation (recommended)

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

The PKGBUILD compiles `rssi-helper` from source and installs all components. Post-install hooks automatically:

- Set `CAP_NET_ADMIN` on `rssi-helper`
- Reload udev rules
- Restart keyd
- Set `fnmode=1` on hid_apple module
- Patch KDE Bluedevil panel (with backup)

## Post-install setup

### 1. User permissions

Add your user to the `input` group for hidraw access:

```bash
sudo usermod -aG input $USER
```

Log out and back in for the group change to take effect.

### 2. Enable services

```bash
# Battery provider daemon (BlueZ Battery1 + telemetry)
systemctl --user enable --now apple-kb-monitor.service

# Brightness daemon (F1/F2 -> DDC/CI + KDE OSD)
systemctl --user enable --now apple-brightness.service
```

### 3. Verify

```bash
# Check services are running
systemctl --user status apple-kb-monitor.service
systemctl --user status apple-brightness.service

# Quick battery check
apple-kb-monitor --once

# Full telemetry
apple-kb-monitor --status
```

## Dependencies

### Required

| Package | Purpose |
|---------|---------|
| `python` (>= 3.10) | Main daemon runtime |
| `python-dbus-fast` | Async D-Bus (BlueZ Battery Provider) |
| `python-dbus` | D-Bus bindings |
| `bluez` (>= 5.56) | Bluetooth stack (Battery Provider API) |
| `dbus` | D-Bus system bus |
| `keyd` (>= 2.5) | System-level key remapping |

### Optional

| Package | Purpose |
|---------|---------|
| `bluez-utils` | `bluetoothctl` CLI for BT management |
| `libnotify` | Desktop notifications on low battery |
| `python-paho-mqtt` | MQTT Home Assistant integration |
| `ddcutil` | Legacy DDC/CI (replaced by ddc-tool for direct I2C) |

## Building ddc-tool from source

The Rust binary is not built by the PKGBUILD (it is a separate component). To build:

```bash
cd ddc-tool
cargo build --release
sudo cp target/release/ddc-tool /usr/bin/ddc-tool
```

### I2C setup for ddc-tool

```bash
# Load i2c-dev module
sudo modprobe i2c-dev

# Make persistent
echo "i2c-dev" | sudo tee /etc/modules-load.d/i2c-dev.conf

# Grant user access to I2C bus
sudo usermod -aG i2c $USER
```

Identify the correct I2C bus for your monitor:

```bash
# List I2C buses
ls /dev/i2c-*

# Test read (try each bus until one works)
ddc-tool read 6 0x10    # read brightness
```

## Building rssi-helper from source

Only needed if building outside of `makepkg`:

```bash
gcc -O2 -Wall -o rssi-helper rssi-helper.c
sudo install -m755 rssi-helper /usr/lib/apple-kb-monitor/rssi-helper
sudo setcap cap_net_admin+ep /usr/lib/apple-kb-monitor/rssi-helper
```

## Plasma widget installation

```bash
# Install the widget
plasmapkg2 -i plasma/com.agenceapi.devicehub

# Or update an existing installation
plasmapkg2 -u plasma/com.agenceapi.devicehub
```

Then add "ApiHub" from the Plasma widget browser to your panel or desktop.

## Bluedevil panel patch

The PKGBUILD installs the patched `DeviceItem.qml` automatically. To apply manually:

```bash
# Backup original
sudo cp /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml \
        /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml.orig

# Apply patch
sudo cp kde/DeviceItem.qml \
        /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml
```

To restore original after uninstall:

```bash
sudo mv /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml.orig \
        /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml
```

## keyd configuration

The keyd config is installed at `/etc/keyd/apple-keyboard.conf` and targets the Apple keyboard by USB vendor/product ID (`05ac:0256`).

keyd must be running as a system service:

```bash
sudo systemctl enable --now keyd
```

To verify the keyboard is detected:

```bash
sudo keyd list
```

## MQTT Home Assistant setup

Install the MQTT client library:

```bash
pip install paho-mqtt
# or
sudo pacman -S python-paho-mqtt
```

Run with MQTT:

```bash
apple-kb-monitor --mqtt 192.168.8.10 --mqtt-port 1883 --mqtt-topic homeassistant
```

This publishes auto-discovery config to Home Assistant and creates three entities:

- `sensor.apple_kb_battery` -- battery percentage
- `sensor.apple_kb_voltage` -- battery voltage (mV)
- `sensor.apple_kb_rssi` -- Bluetooth RSSI (dBm)

## Waybar integration

Add to your Waybar config:

```json
"custom/apple-kb": {
    "exec": "apple-kb-monitor --waybar",
    "return-type": "json",
    "interval": 60
}
```

## Troubleshooting

### hidraw permission denied

```bash
# Verify udev rules are loaded
udevadm control --reload-rules
udevadm trigger

# Check your groups
groups

# Verify hidraw permissions
ls -la /dev/hidraw*
```

### BlueZ Battery Provider not showing in KDE

```bash
# Check service logs
journalctl --user -u apple-kb-monitor.service -f

# Verify Battery1 interface is registered on D-Bus
busctl tree org.bluez
```

### RSSI returns null

```bash
# Verify capabilities
getcap /usr/lib/apple-kb-monitor/rssi-helper

# Test manually
/usr/lib/apple-kb-monitor/rssi-helper AA:BB:CC:DD:EE:FF
```

### ddc-tool permission denied on /dev/i2c-*

```bash
# Check i2c group
groups | grep i2c

# Add to group if missing
sudo usermod -aG i2c $USER
# Log out and back in
```

### keyd keys not working

```bash
# Check keyd service
sudo systemctl status keyd

# Reload config
sudo keyd reload

# Verify device detection
sudo keyd list
```
