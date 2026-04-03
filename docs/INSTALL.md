# Installation guide

## Prerequisites

- Arch Linux or Manjaro (PKGBUILD provided)
- Bluetooth adapter supported by BlueZ
- Apple Wireless Keyboard paired via `bluetoothctl`
- For DDC/CI: I2C bus accessible (`i2c-dev` module loaded)
- For Plasma widget: KDE Plasma >= 6.0

## Package installation

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

The PKGBUILD compiles two Rust binaries (`apihub-app` and `ddc-tool`) and installs all system integration files. Post-install hooks automatically:

- Reload udev rules (hidraw group permissions)
- Restart keyd (special key mapping)
- Set `fnmode=1` on the hid_apple kernel module
- Patch KDE Bluedevil panel (with backup of the original)

### Build dependencies

| Package | Purpose |
|---------|---------|
| `rust` | Compile apihub-app and ddc-tool |
| `gcc` | Linker for Rust builds |

### Runtime dependencies

| Package | Required | Purpose |
|---------|----------|---------|
| `bluez` | yes | Bluetooth stack (Battery Provider API) |
| `keyd` | yes | System-level key remapping (Wayland-compatible) |
| `bluez-utils` | optional | `bluetoothctl` CLI for BT management |
| `libnotify` | optional | Desktop notifications on low battery |
| `mosquitto` | optional | MQTT broker (local or for testing) |

## Post-install setup

### 1. User permissions

Add your user to the `input` group for hidraw access:

```bash
sudo usermod -aG input $USER
```

Log out and back in for the group change to take effect.

For DDC/CI monitor control, also add yourself to the `i2c` group:

```bash
sudo usermod -aG i2c $USER
```

### 2. I2C setup for DDC/CI

Load the i2c-dev kernel module (if not already loaded):

```bash
sudo modprobe i2c-dev

# Make persistent across reboots
echo "i2c-dev" | sudo tee /etc/modules-load.d/i2c-dev.conf
```

apihub-app auto-detects the correct I2C bus. To verify manually:

```bash
ddc-tool read 6 0x10    # try reading brightness from bus 6
```

### 3. Configuration

Copy the example config and edit as needed:

```bash
mkdir -p ~/.config/apple-kb-monitor
cp /etc/apple-kb-monitor/config.toml.example ~/.config/apple-kb-monitor/config.toml
```

Edit `~/.config/apple-kb-monitor/config.toml`:

```toml
[ddc]
bus = "/dev/i2c-6"           # I2C bus (auto-detected if omitted)

[mqtt]
broker = "192.168.8.3"       # MQTT broker address (leave empty to disable)
port = 1883
user = ""
password = ""
topic_prefix = "homeassistant"

[monitor]
model = "lg_34gn850"         # Used in MQTT topic path

[brightness]
min = 2                      # DDC brightness floor (%)
max = 70                     # DDC brightness ceiling (%)
lamp_entity = "light.bureau" # HA entity to sync with
```

### 4. Launch

**apihub-app** is a graphical desktop application. Launch it from:

- KDE application menu: search for **ApiHub**
- Terminal: `apihub-app`

It is not a background service -- it runs as a normal desktop application with a window.

### 5. Optional: legacy CLI daemon

The Python `apple-kb-monitor` CLI can run as a systemd user service for headless BlueZ Battery Provider and MQTT publishing:

```bash
systemctl --user enable --now apple-kb-monitor.service
```

Verify:

```bash
systemctl --user status apple-kb-monitor.service
apple-kb-monitor --once    # quick battery check
apple-kb-monitor --status  # full telemetry dump
```

### 6. keyd verification

keyd must be running as a system service:

```bash
sudo systemctl enable --now keyd
sudo keyd list              # verify keyboard detection
```

The config at `/etc/keyd/apple-keyboard.conf` targets the Apple keyboard by USB vendor/product ID (`05ac:0256`).

## Plasma widget

The widget is installed system-wide by the PKGBUILD. To install manually:

```bash
plasmapkg2 -i plasma/com.agenceapi.devicehub

# Or update an existing installation
plasmapkg2 -u plasma/com.agenceapi.devicehub
```

Add "ApiHub" from the Plasma widget browser to your panel or desktop.

## Bluedevil panel patch

The PKGBUILD patches the Bluedevil DeviceItem.qml automatically (with backup). To apply manually:

```bash
sudo cp /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml \
        /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml.orig

sudo cp /usr/share/apple-kb-monitor/kde/DeviceItem.qml \
        /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml
```

To restore the original after uninstall:

```bash
sudo mv /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml.orig \
        /usr/share/plasma/plasmoids/org.kde.plasma.bluetooth/contents/ui/DeviceItem.qml
```

## Troubleshooting

### hidraw permission denied

```bash
udevadm control --reload-rules && udevadm trigger
groups                       # verify "input" is listed
ls -la /dev/hidraw*          # check permissions
```

### DDC/CI permission denied on /dev/i2c-*

```bash
groups | grep i2c            # verify "i2c" group membership
sudo modprobe i2c-dev        # verify module is loaded
ls -la /dev/i2c-*            # check permissions
```

### keyd keys not working

```bash
sudo systemctl status keyd
sudo keyd reload
sudo keyd list               # verify device detection
```

### BlueZ Battery Provider not showing in KDE

```bash
journalctl --user -u apple-kb-monitor.service -f
busctl tree org.bluez        # verify Battery1 interface is registered
```

## Uninstall

```bash
sudo pacman -R apple-kb-monitor
```

The post_remove hook restores the original Bluedevil QML and reloads udev rules. Disable user services manually:

```bash
systemctl --user disable apple-kb-monitor.service
```
