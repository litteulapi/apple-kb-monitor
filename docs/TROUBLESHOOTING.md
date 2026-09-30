# Troubleshooting

See also the Troubleshooting section of [INSTALL.md](INSTALL.md). Start with the **Diag** tab of apihub-app.

| Symptom | Cause / check | Fix |
|---|---|---|
| No keyboard data, `Permission denied` on `/dev/hidraw*` | User not in `input`, udev rule not loaded | `sudo usermod -aG input $USER`, re-login; `sudo udevadm control --reload-rules && sudo udevadm trigger` |
| Display tab empty / DDC errors | `i2c-dev` not loaded, user not in `i2c`, wrong bus | `sudo modprobe i2c-dev`; `sudo usermod -aG i2c $USER`; set `[ddc] bus` or rely on auto-detect; test with `ddc-tool read 6 0x10` |
| DDC values stale or shifted on NVIDIA | Pipeline aliasing; the driver already does a double-read, and writes use `I2C_SLAVE` + `write()` | Check the bus is the DP/HDMI adapter of the monitor, not another adapter |
| RSSI / TX power missing | BlueZ MGMT needs `CAP_NET_ADMIN`; `rssi.rs` returns nothing on privilege failure | Run with the capability (or via sudo) if you need it |
| Battery not shown in KDE | BlueZ Battery Provider not registered | Check the D-Bus policy `/etc/dbus-1/system.d/com.agenceapi.AppleKbMonitor.conf`, then `bluetooth.service` and the app logs (`journalctl --user`, or run `apihub-app` in a terminal) |
| F1/F2 do nothing | keyd virtual keyboard not present | `systemctl status keyd`; `/etc/keyd/apple-keyboard.conf` |
| Media keys inverted | `hid_apple.fnmode` | `cat /sys/module/hid_apple/parameters/fnmode` should be 1 |
| Home Assistant entity unavailable after broker restart | Discovery lost | apihub-app republishes discovery on each MQTT reconnect; the Python bridge does the same since the 2026-08 fix |
| Two processes fight over MQTT / brightness | `mqtt-bridge.service` or `apple-brightness.service` running with apihub-app | Stop the legacy unit: `systemctl --user disable --now mqtt-bridge.service` |
| Bluetooth disconnections while polling | Commit 61369b2 fixed HID reads causing disconnections | Update to the latest build; attach `apple-kb-monitor --dump` output to an issue if it persists |

Logs: apihub-app prints `[ddc] ...` and similar prefixes on stderr.
