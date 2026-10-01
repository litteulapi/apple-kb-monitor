# Troubleshooting

See also the Troubleshooting section of [INSTALL.md](INSTALL.md). Start with the **Diag** tab of apihub-app (binaries, user service, `hidraw readable`, keyd config, udev rules, `hid_apple fnmode`, RSSI helper).

| Symptom | Cause / check | Fix |
|---|---|---|
| No keyboard data, `Permission denied` on `/dev/hidraw*` | `uaccess` ACL not applied: rule not loaded, keyboard connected before the install, or session is not the active seat (SSH) | `sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw`, then reconnect the keyboard. No `input` group is needed. |
| Wired Apple keyboard has no hidraw access | Rule must match `0003:05AC:*` (issue #73) | Update to a build with the 3-pattern rule `0005:05AC:*\|0005:004C:*\|0003:05AC:*` |
| RSSI / TX power missing | `rssi-helper` lacks `cap_net_admin`, is missing, or timed out (1.5 s); `rssi.rs` returns nothing and logs the reason once | `getcap /usr/lib/apple-kb-monitor/rssi-helper`; `sudo setcap cap_net_admin+ep /usr/lib/apple-kb-monitor/rssi-helper`; run the helper by hand with the keyboard MAC |
| Battery percentage differs from the HID value | The displayed percentage is the kernel `power_supply` value (what UPower shows); raw HID values are diagnostics | `tests/live/check_keyboard.sh` compares sysfs, UPower, BlueZ and the CLI |
| Battery not shown in KDE | For keyboards with a kernel `power_supply`, KDE shows the kernel one and UPower hides the BlueZ duplicate (expected). Otherwise the BlueZ Battery Provider is not registered. | `busctl tree org.bluez` and look for `Battery1`; check `/etc/dbus-1/system.d/com.agenceapi.AppleKbMonitor.conf`, `bluetooth.service`, and the app logs (`journalctl --user`, or run `apihub-app` in a terminal) |
| Battery disappears after `bluetoothd` restart | Provider registration is redone automatically | Look for `[bluez] bluetoothd gone, will re-register` then `[bluez] registered battery provider` in the app logs |
| Tray icon missing | plasmashell started after the app, or was restarted | The tray re-registers when the StatusNotifierWatcher reappears (issue #74); otherwise restart `apihub-app` |
| Special keys do nothing | keyd not running or wrong device | `systemctl status keyd`; `/etc/keyd/apple-keyboard.conf` targets `05ac:0256` |
| Media keys inverted | `hid_apple.fnmode` | `cat /sys/module/hid_apple/parameters/fnmode` should be 1 |

Logs: apihub-app prints prefixed lines (`[tray]`, `[apihub]`, ...) on stderr.
