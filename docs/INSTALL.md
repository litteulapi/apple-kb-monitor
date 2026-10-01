# Installation guide

## Prerequisites

- Arch Linux or Manjaro (PKGBUILD provided)
- Bluetooth adapter supported by BlueZ
- Apple Bluetooth keyboard paired via `bluetoothctl`
- For the Plasma widget: KDE Plasma >= 6.0

## Package installation

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

The PKGBUILD compiles the Rust binary `apihub-app` and the C helper `rssi-helper`, and installs all system integration files. The `post_install` hook automatically:

- Reloads udev rules and re-triggers hidraw devices (applies the `uaccess` ACL to keyboards already connected)
- Restarts keyd (special key mapping)
- Sets `fnmode=1` on the hid_apple kernel module
- Applies `setcap cap_net_admin+ep /usr/lib/apple-kb-monitor/rssi-helper` (RSSI); on failure it prints the command to run by hand
- Patches the KDE Bluedevil panel (with backup of the original)

### Build dependencies

| Package | Purpose |
|---------|---------|
| `rust` | Compile apihub-app |
| `gcc` | Compile rssi-helper, linker for Rust builds |

### Runtime dependencies

| Package | Required | Purpose |
|---------|----------|---------|
| `python`, `python-dbus-fast` | yes | Python CLI |
| `bluez` | yes | Bluetooth stack (Battery Provider API) |
| `keyd` | yes | System-level key remapping (Wayland-compatible) |
| `bluez-utils` | optional | `bluetoothctl` CLI for BT management |
| `libnotify` | optional | Desktop notifications on low battery |

## Post-install setup

### 1. User permissions

None. The udev rule `70-apple-kb-hidraw.rules` tags Apple hidraw devices `uaccess`; logind gives the user of the active seat an ACL. There is no `input` group step. If the keyboard was connected before the install and `/dev/hidraw*` is still not readable, reconnect it or run `sudo udevadm trigger --subsystem-match=hidraw`.

Check:

```bash
getfacl /dev/hidraw* 2>/dev/null | grep -B3 "user:$USER"
```

### 2. RSSI

RSSI and TX power come from `/usr/lib/apple-kb-monitor/rssi-helper`, the only binary with `cap_net_admin`. Check:

```bash
getcap /usr/lib/apple-kb-monitor/rssi-helper     # cap_net_admin=ep
/usr/lib/apple-kb-monitor/rssi-helper AA:BB:CC:DD:EE:FF   # {"rssi":-5,"tx_power":4,...}
```

Without the capability the app still works and shows no RSSI. Re-apply with `sudo setcap cap_net_admin+ep /usr/lib/apple-kb-monitor/rssi-helper` (pacman rewrites the file on every upgrade; `post_install` re-applies it).

### 3. Configuration

None. See [CONFIGURATION.md](CONFIGURATION.md).

### 4. Launch

**apihub-app** is a desktop application with a tray icon (click the scarab icon, or use "Show Window", to open the window). Launch it from:

- KDE application menu: search for **ApiHub**
- Terminal: `apihub-app`

### 5. Optional: Python CLI daemon

The Python `apple-kb-monitor` CLI can run as a systemd user service for a headless BlueZ Battery Provider and low-battery notifications:

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
sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
udevadm info -q property /dev/hidrawN | grep TAGS    # must contain :uaccess:
ls -la /dev/hidraw*                                  # ACL "+" and your user
```

The ACL is granted to the user of the active seat only: an SSH session does not get it.

### No RSSI

```bash
getcap /usr/lib/apple-kb-monitor/rssi-helper
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
