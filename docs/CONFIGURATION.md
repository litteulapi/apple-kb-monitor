# Configuration

There is no configuration file. `config.toml` and `config.toml.example` belonged to the removed display/DDC/MQTT feature set and no code in this repository reads them any more (a leftover `~/.config/apple-kb-monitor/config.toml` is ignored and can be deleted).

## What is configurable

| Item | Where | Notes |
|---|---|---|
| RSSI helper location | env `APPLE_KB_RSSI_HELPER` | Development/test override of `/usr/lib/apple-kb-monitor/rssi-helper` (`rssi.rs`). Not a privilege boundary: the helper itself carries the capability. |
| Low-battery threshold, poll interval | Python CLI `--threshold` (default 15), `--interval` (default 300) | Set in `systemd/apple-kb-monitor.service` for the user daemon |
| `fnmode` | `/etc/modprobe.d/hid_apple.conf` (`options hid_apple fnmode=1`) | Applied immediately by `post_install`. Change it with `akmctl set fnmode N --persist`. The Diag tab of `apihub-app` shows the applied value (`/sys/module/hid_apple/parameters/fnmode`) and the configured one (`/etc/modprobe.d`), and flags a difference |
| Service to enable | `systemctl --user enable --now apple-kb-monitord.service` | The Rust daemon; the Python `apple-kb-monitor.service` conflicts with it, enable only one |
| Special keys | `/etc/keyd/apple-keyboard.conf` (kept on upgrade, `backup=`) | Device `05ac:0256` |
| uaccess rule | `/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules` | Override by placing a file of the same name in `/etc/udev/rules.d/` |

## Files written by the app

| Path | Content |
|---|---|
| `~/.local/share/apple-kb-monitor/history.jsonl` | Battery history (timestamp, percentage, voltage); invalid points are not written |

The Python CLI keeps its own state under `$XDG_RUNTIME_DIR/apple-kb-monitor/`.
