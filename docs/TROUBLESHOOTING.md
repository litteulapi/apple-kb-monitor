# Troubleshooting

Two commands answer most questions, both read-only:

```bash
akmctl doctor          # link: adapter, pairing, link key (with sudo), hidraw, adapter power management,
                       # BlueZ / UPower configuration, bluetoothd journal (last 6 h + this boot), daemon
akmctl selftest        # health: daemon, freshness, versions, link, journals, crashes, window, disk, history
```

Every `doctor` finding comes with the command that fixes it. The System Settings module ("Apple Keyboard" → Diagnostics) runs both and has a "Copy the diagnosis" button for a ticket. The window's **Diag** tab checks binaries, services, hidraw, keyd, udev rules, `hid_apple fnmode` and the RSSI helper. Logs: `journalctl --user -u apple-kb-monitord.service -b`.

## The keyboard does not reconnect

| Symptom | Cause | What to do |
|---|---|---|
| The keyboard is silent after a while; `bluetoothctl connect` fails with `br-connection-create-socket` or `Host is down` | Not the pairing. The link dropped on a radio silence of the keyboard (supervision timeout, often during a burst of battery requests), the keyboard stopped listening, and BlueZ gave up paging after 2-3 min ([RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) §4) | **Press a key.** If nothing happens within a few seconds, **switch the keyboard off and on** (the light comes on at power-up) — do not skip this step, a key press alone did not bring it back in the logged episodes. Then `akmctl repair`: it wakes and reconnects first; re-pairing is offered only after a typed confirmation and the pairing is never removed otherwise |
| `akmctl repair` says the keyboard does not answer | Keyboard asleep, off, or batteries empty | Switch it off and on, check the batteries (`akmctl status`), run `akmctl repair` again |
| The keyboard drops every night / after sleep | The Bluetooth adapter went into USB autosuspend; `doctor` line `adapter-pm` | `sudo install -m644 udev/61-akm-bt-adapter-no-autosuspend.rules /etc/udev/rules.d/` (Intel `8087:0026`; adapt the ids), `sudo udevadm control --reload`, then re-plug or reboot. The package does not install this rule |
| `doctor`: `bluez-conf` not in place | `FastConnectable` / `[Policy] Reconnect*` missing in `/etc/bluetooth/main.conf` | `sudo python3 bluetooth/akm-conf.py bluez --apply && sudo systemctl restart bluetooth` |
| `doctor`: `upower` polls every 30 s | UPower sends 2 GET_REPORT per 30 s; the keyboard never sleeps | optional: `sudo python3 bluetooth/akm-conf.py upower --apply && sudo systemctl restart upower` (the daemon stays the battery source through the BlueZ provider) |
| Pairing refused or missing ("no usable pairing") | Link key lost on the host side (Plasma "Forget", `bluetoothctl remove`, or BlueZ could not write `/var/lib/bluetooth`) | `akmctl repair` (pairing assistant). If `doctor` mentions disk space, see below first |
| `doctor` warns about **disk space** | `/var/lib/bluetooth` on a full filesystem: `bluetoothd` cannot store the link key, the pairing silently fails or is refused | free space (btrfs: check the metadata too), verify with `sudo akmctl doctor`, then `akmctl repair` if the keyboard is still refused. The self-check reports `disk` below 1 GiB / 5 % (warning) and 100 MiB / 1 % (grave) for `/var/lib/bluetooth` and `~/.local/state/apple-kb-monitor` |
| `doctor`: `bluetoothd journal not readable` | System journal needs group `systemd-journal` | `sudo usermod -aG systemd-journal $USER` (log in again) or `sudo akmctl doctor` |
| `bluetoothd` errors in the journal (`journal-bluetooth` in the self-check) | BlueZ-side failures: `br-connection-*`, `Host is down`, `Input/output error` | `journalctl -b -u bluetooth --since -6h`; `akmctl doctor` interprets the known ones |
| Keyboard "forgotten" from Plasma: the daemon still showed it | Fixed in 3.1.0-11 (#252): removed at once, one `KeyboardRemoved` notification with "Repair…" | update, or `systemctl --user restart apple-kb-monitord.service` |
| The keyboard is "switched off" in the notifications although it only lost the link | Input `0x13` bit 1 = 0 is the keyboard's own power-off message (as macOS reads it); a link loss says "disconnected" | nothing to do |

## Battery, telemetry

| Symptom | Cause | What to do |
|---|---|---|
| The percentage never moves, then drops by a step | Expected: the keyboard's `0x47` is a firmware interpolation that only steps down at reconnections ([CONFIGURATION.md](CONFIGURATION.md)). The daemon shows the **age** of the reading and never rewrites the value | read the **estimate** (voltage projected on the chemistry curve) and set `[battery] chemistry` to your cells |
| Alerts at 64 / 40 / 25 % of the keyboard's own percentage | The thresholds 30 / 15 / 5 % apply to the **estimated** charge; the notification says "Estimated charge" or "Keyboard indication" | adjust `[alerts] thresholds`, or `chemistry = "unknown"` to alert on the keyboard's percentage |
| Two low-battery notifications, one from KDE | PowerDevil warns about the same keyboard; by default the daemon then sends one `BatteryEstimate` reminder only (#254) | `[notifications] defer_to_powerdevil = false` to get the full alerts anyway |
| `Updated: N s ago` grows, no new reading | Apple's schedule: one read 60 s after the connection, then every 4 h (1 h after a failure); a read also needs the keyboard to answer | `akmctl status --json` → `kb_error`; the tray "Refresh" is bounded to one per 5 min; press a key before `akmctl dump` |
| `Voltage: n/a` or history without `mv_0x46` | The voltages are read only in the daemon's bursts; a failed or breaker-blocked burst leaves them empty | wait for the next burst; `akmctl info` shows what was read and when |
| `Firmware: not read yet` / `Status: unknown` | `0x4F` is read once per connection, after the routine burst; `unknown` also means a model absent from the table | reconnect the keyboard; [FIRMWARE.md](FIRMWARE.md) |
| `On kb: n/a` | The name stored in the keyboard (`0x51`-`0x54`) is read once per connection, in low priority | wait, or reconnect; `akmctl rename --device-name --show` |
| Nothing is read, journal says the breaker is open | 3 consecutive silences: Apple's rule, nothing goes out until a new connection (or a sleep) | switch the keyboard off and on; `cat /run/user/$UID/apple-kb-monitor/breaker.state` |
| `Permission denied` on `/dev/hidraw*` | `uaccess` ACL not applied: rule not loaded, keyboard connected before the install, or the session is not the active seat (SSH, another TTY) | `sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw`, reconnect the keyboard; `getfacl /dev/hidraw*` must list your user. No group is needed for the keyboard |
| Battery not shown by KDE's Bluetooth applet | When the kernel provides `power_supply`, UPower shows that one and hides the BlueZ duplicate: expected | `busctl tree org.bluez` shows `Battery1` if the provider is registered; `journalctl --user -u apple-kb-monitord.service | grep bluez` |

## Signal

| Symptom | Cause | What to do |
|---|---|---|
| `Signal: n/a` | The **daemon** does not have the group `akm`: `rssi-helper` is `root:akm 0750` since 3.1.0-7 (#209). Verified on 2026-10-01 (user outside `akm`) and 2026-10-03 (user in `akm`, but the daemon's `Groups:` without its GID: the user manager predates the change, `Linger=yes`) | `sudo usermod -aG akm $USER`; it reaches the daemon only when `user@UID.service` restarts: log out of every session, or with `Linger=yes` reboot / `sudo systemctl restart user@$(id -u).service` from a text console ([INSTALL.md](INSTALL.md)); check `grep Groups /proc/$(pgrep -x apple-kb-monitord)/status`; `getcap /usr/lib/apple-kb-monitor/rssi-helper` must print `cap_net_admin=ep` (else `sudo setcap cap_net_admin+ep /usr/lib/apple-kb-monitor/rssi-helper`) |
| A value such as `0` or `-3` without a unit | Correct: BR/EDR RSSI is the gap in dB to the controller's golden receive range, 0 = ideal, not a power level (#174) | nothing to do |
| Helper exit code 2 / 3 / 4 / 5 | socket, MGMT status, timeout (1.5 s), connection info unavailable | `/usr/lib/apple-kb-monitor/rssi-helper <MAC>` by hand; the keyboard must be connected |

## Keys

| Symptom | Cause | What to do |
|---|---|---|
| F4 (Launchpad) or Eject does nothing | Plasma binds nothing to `XF86LaunchB` / `XF86Eject` by default; the package changes no KDE shortcut | `akmctl keys --check` shows `[--]` on the key; `akmctl keymap kde-apply --dry-run`, then `kde-apply` (`--undo` removes exactly those keys) |
| F-keys and media keys inverted | `hid_apple fnmode` (1 = media keys by default, 2 = F-keys first) | `akmctl get fnmode`; `akmctl set fnmode 2 --persist` (polkit dialog) |
| J K L U I O M 7 8 9 type digits | NumLock emulation of `hid_apple` after F6 (`KEY_NUMLOCK`) | press F6 again; `akmctl keymap set F6 KEY_F6` then `apply` to drop it |
| Keys stopped working after an upgrade; keyd crashed (`SEGV`) | `keyd reload` crashes keyd 2.6.0 (dangling `active_kbd`, [KEYD.md](KEYD.md)); the package has not reloaded keyd since 3.1.0-10 | `sudo systemctl restart keyd` (never `keyd reload`); keyd is optional: `sudo systemctl disable --now keyd` and use `akmctl keymap` instead |
| A remapped key lost its Fn layer | `hid_apple` looks the Fn table up **after** the hwdb: a code outside the table has no Fn variant | [TOUCHES.md](TOUCHES.md) §4-5; `akmctl keymap unset <KEY>` then `apply` |
| `akmctl set` / `keymap apply` says "not authorized" | Polkit `auth_admin`, local active session only (no SSH, no inactive seat) | run from the graphical session; the dialog asks for an administrator password every time (`_keep` is deliberately not used) |
| The kernel name of the keyboard did not change after `akmctl rename` | The alias is BlueZ's; KWin and System Settings → Keyboard show the kernel name, which is re-read at the next connection | switch the keyboard off and on |

## Daemon, tray, window, package

| Symptom | Cause | What to do |
|---|---|---|
| No tray icon, no alerts after login | `apple-kb-monitord.service` not running (masked, failed, or an upgrade left the old binary) | `systemctl --user status apple-kb-monitord.service`; `systemctl --user restart apple-kb-monitord.service`; the package enables the unit for every user through its own link `/usr/lib/systemd/user/default.target.wants/` (never `systemctl --global`, #260); after an upgrade the scriptlet lists the processes still on the old binary |
| Self-check `versions`: daemon ≠ akmctl ≠ package | The daemon was not restarted after the upgrade | `systemctl --user restart apple-kb-monitord.service` |
| The icon opens a small popup instead of the window, right click has no menu (3.1.0-22 to 25) | The widget had taken over the notification area without the daemon's menu (#253, #267) | update to 3.1.0-26 or later, then restart plasmashell (`systemctl --user restart plasma-plasmashell.service`): the widget is the icon, left click = anchored panel, right click = the daemon's full menu (#268) |
| Widget old after an upgrade, or not offered in the notification area | plasmashell keeps the widget it loaded at its start | `systemctl --user restart plasma-plasmashell.service`; then notification area → Configure → Entries → ApiHub |
| Global shortcuts "Apple Keyboard" missing in System Settings → Shortcuts | kglobalaccel reads `/usr/share/kglobalaccel` at session start | log out and back in once after the install |
| Tray icon gone after `plasmashell` restarted | The daemon re-registers when the StatusNotifierWatcher reappears | wait a few seconds; `systemctl --user restart apple-kb-monitord.service` otherwise |
| The window says "Not responding" / the self-check reports `ui-heartbeat` | Fixed in 3.1.0-8 (#230-#236): vsync, D-Bus calls bounded, history off the UI thread; the window writes a heartbeat file the self-check watches | update; `akmctl selftest` reports `ui-heartbeat` grave when the heartbeat is older than 10 s |
| `akmctl` exit code 2 | Daemon not on the session bus | `systemctl --user start apple-kb-monitord.service` (the unit) or just open the window / widget (D-Bus activation) |
| `config.toml` warnings in the journal | Unknown or mistyped key: the default is kept | fix the key ([CONFIGURATION.md](CONFIGURATION.md)); sections of another program sharing the file are ignored silently since 3.1.0-13 |
| `WillShutdown` not sent at shutdown | Option off, keyboard disconnected, breaker open, or the user unit masked | `akmctl status` line `Shutdown:`; `systemctl --user status apple-kb-monitor-shutdown.service`; `[apple] will_shutdown` |
| Sleep / wake bytes not sent (`NOT sent` in the system journal) | `enabled = false` in `/etc/apple-kb-monitor/hid-suspend.conf`, file not root-owned, units masked, or the daemon's breaker is open / stale | `akmctl hid-control suspend --dry-run` (polkit) shows the socket that would be used; `journalctl -b -u apple-kb-monitor-suspend -u apple-kb-monitor-resume` |
| `post_install` warned about `setcap` or the group `akm` | `libcap` missing, or `systemd-sysusers` failed | `sudo systemd-sysusers /usr/lib/sysusers.d/apple-kb-monitor.conf`, `sudo chgrp akm /usr/lib/apple-kb-monitor/rssi-helper && sudo chmod 0750 … && sudo setcap cap_net_admin+ep …` (in this order: `chgrp`/`chmod` drop the capability) |
| History shows `voltage_valid: false` on old lines | Lines written before schema 2 carry no measured voltage; they are kept for the percentage and ignored for the estimate | nothing to do; `history.jsonl.pre-schema2` is the untouched copy |

## Reporting a problem

One Gitea issue per bug, label `bug`: https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues. Attach `akmctl doctor --json`, `akmctl selftest --json --no-save`, `akmctl status --json` and the daemon journal. The self-check can open the issue itself for a new grave problem (`AKM_SELFCHECK_ARGS=--gitea-issue` in `~/.config/apple-kb-monitor/selfcheck.env`, off by default).
