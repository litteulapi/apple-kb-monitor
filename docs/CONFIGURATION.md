# Configuration

Checked against `apihub-app/akm-core/src/config.rs` (parser and defaults), the systemd units and `PKGBUILD` of 3.1.0-15.

## `config.toml` of the daemon

`apple-kb-monitord` reads one optional file, `$XDG_CONFIG_HOME/apple-kb-monitor/config.toml` (default `~/.config/apple-kb-monitor/config.toml`), **once at startup** (`systemctl --user restart apple-kb-monitord.service` after an edit; the System Settings module offers the restart). A missing file means the defaults. The parser reads a small TOML subset: sections, integers, floats, booleans, quoted strings, arrays of integers (possibly over several lines), `#` comments (not inside a quoted string), an optional UTF-8 BOM. An unknown key or a mistyped value is **logged as a warning and ignored, the default is kept**; four section names reserved by another program that shares this file are skipped without a warning (`FOREIGN_SECTIONS` in `config.rs`).

Every key, with its default:

```toml
[alerts]
thresholds = [30, 15, 5]   # % of the ESTIMATED charge (else of the keyboard's %), 1..99 each; one notification per threshold crossed
hysteresis = 3             # points above the threshold before it is re-armed (integer or float)
critical = 5               # thresholds <= critical are sent with the "critical" urgency (0..99)
enabled = true             # false: no battery alert at all

[battery]
chemistry = "alkaline"     # "alkaline" | "nimh" | "lithium" | "unknown" (#178)

[notifications]
connection = true          # "disconnected" / "reconnected (N %)" / "switched off", low urgency
battery_replaced = true    # "new batteries detected"
defer_to_powerdevil = true # KDE PowerDevil already warns about this keyboard: one distinct reminder only (#254)

[display]
apple_percent = true       # also show the percentage "as macOS shows it", labelled "Apple display" (#213)

[apple]
will_shutdown = true           # send WillShutdown (Feature 0x40) once at shutdown / restart, as macOS (#191)
disconnect_on_breaker = true   # after 3 unanswered requests: ask BlueZ once to disconnect, as macOS (#251); false = only stop the requests
# allow_device_name_write: OBSOLETE (#248), still read, ignored; may be deleted
```

| Section | Key | Type | Default | Read by | Effect |
|---|---|---|---|---|---|
| `[alerts]` | `thresholds` | array of integers 1..99 | `[30, 15, 5]` | daemon | Values outside 1..99 are dropped with a warning; an empty array disables the alerts (warning). |
| `[alerts]` | `hysteresis` | integer or float | `3` | daemon | A crossed threshold is re-armed when the value climbs back above threshold + hysteresis. |
| `[alerts]` | `critical` | integer 0..99 | `5` | daemon | Urgency of the notification: critical (persistent) at or below, normal above. |
| `[alerts]` | `enabled` | bool | `true` | daemon | Master switch of the percentage alerts. The keyboard-driven alerts (Input `0x30`) are separate. |
| `[battery]` | `chemistry` | string | `"alkaline"` | daemon, window, widget, module | Discharge curve of the estimate (see below). `"unknown"` = no estimate, alerts on the keyboard's percentage. |
| `[notifications]` | `connection` | bool | `true` | daemon | Link notifications (`KeyboardDisconnected`, `KeyboardReconnected`, `KeyboardOff`). |
| `[notifications]` | `battery_replaced` | bool | `true` | daemon | `BatteryReplaced` when fresh cells are detected. |
| `[notifications]` | `defer_to_powerdevil` | bool | `true` | daemon | When PowerDevil shows its own low-battery warning for this keyboard, the 30 / 15 / 5 % alerts shrink to one `BatteryEstimate` reminder ([INTEGRATION-KDE.md](INTEGRATION-KDE.md) §3). |
| `[display]` | `apple_percent` | bool | `true` | daemon (JSON), tray, window, widget, `akmctl status` | Secondary figure only; alerts and the estimate never use it. IOBluetooth remaps the raw `0x47` value: 54..100 → 100 %, 21..53 → 21 + (raw − 21) × 2.4375, below 21 unchanged. |
| `[apple]` | `will_shutdown` | bool | `true` | daemon, `akmctl status` (`Shutdown:` line, JSON `will_shutdown`) | See "WillShutdown" below. |
| `[apple]` | `disconnect_on_breaker` | bool | `true` | daemon | After the third consecutive silence of the keyboard, one `org.bluez.Device1.Disconnect` (never `RemoveDevice`), bounded to 5 s, never retried. |
| `[apple]` | `allow_device_name_write` | bool | `false` | nobody | **Obsolete.** Still read so that an existing file raises no warning; ignored. See "Name stored in the keyboard" below. |

The **System Settings module** ("Apple Keyboard" → Notifications page) writes this file atomically (`QSaveFile`), rewriting only the keys it changed and keeping comments and unknown keys.

### WillShutdown (`[apple] will_shutdown`, #191)

Default **true**, because it is what macOS sends to this keyboard at every shutdown and restart. When the computer shuts down or restarts, the daemon sends **one** Feature report, `0x40` (`WillShutdown`), made of the report id alone (`53 40` on the Bluetooth link), exactly as Apple's driver does. Conditions, all required: option on, keyboard connected, nothing sent yet in this run, circuit breaker closed, at least 1 s since the previous hardware access. A failure is logged and never retried, and never delays the shutdown by more than 3.5 s. Two doors lead to the same single write: the logind `PrepareForShutdown` delay inhibitor held by the daemon, and the user unit `apple-kb-monitor-shutdown.service` (`ExecStop=akmctl shutdown-notify --only-if-stopping`). `systemctl --user mask apple-kb-monitor-shutdown.service` disables the unit; the inhibitor stays governed by the option. `false` sends nothing. Evidence and context: [PARITE-APPLE.md](PARITE-APPLE.md).

### Name stored in the keyboard (`[apple] allow_device_name_write`, obsolete, #248)

This key was a lock of `akmctl rename --device-name`. It is **obsolete**: the key is still read (no warning) and ignored, whatever its value; it can be deleted. Writing the name stored in the keyboard needs no configuration: `akmctl rename --device-name <name>` asks one confirmation (or takes `--yes`), see [RENOMMER-CLAVIER.md](RENOMMER-CLAVIER.md). The alias on this computer (`akmctl rename <name>`, BlueZ `Alias`) writes nothing to the keyboard.

## Battery: what is shown and what the alerts use

The keyboard reports a percentage (report `0x47`, = kernel `power_supply`). It is **not a remaining charge**: the firmware interpolates the pair voltage on a factory table (2954 / 2506 / 2404 / 2054 mV for 100 / 75 / 50 / 25 %, read in `0x5A`; the thresholds Full / Low / Critical / Empty of `0x60` are read once per connection, #215). On alkaline cells that scale is optimistic in the middle of the battery's life: firmware 75 % is about 35 % of real charge, 50 % about 25 %, 30 % about 7 % ([VERIF-BATTERIE.md](VERIF-BATTERIE.md) §2). It is shown as **"keyboard indication"** and never rewritten.

Next to it the daemon shows an **estimate** (#178): the voltage `0x49` (else `0x46`) projected on a discharge curve of the chemistry declared in `[battery] chemistry`, with a range (±10 points for alkaline, ±15 for NiMH and lithium). The curves are a **[hypothèse]**: read by eye from the Energizer E91 and L91 datasheets and from Eneloop-type NiMH curves at a low drain (~1 mA, 21 °C), ±0.03 V per cell, plus about ±50 mV per pair of ADC calibration. Pair voltage to real charge, alkaline, indicative:

| Pair (mV) | 2950 | 2775 | 2600 | 2506 | 2404 | 2200 | 2124 | 2054 |
|---|---|---|---|---|---|---|---|---|
| Firmware % | 99 | 90 | 80 | 75 | 50 | 35 | 30 | 25 |
| Alkaline estimate | ~84 | ~62 | ~44 | ~35 | ~25 | ~10 | ~7 | ~5 |

* `alkaline` (default; also zinc-carbon): the only chemistry consistent with the 2.97-2.99 V measured a few hours after a battery change.
* `nimh` (Eneloop type): a flat plateau at 1.20-1.25 V per cell; the keyboard's own percentage stays at 50-75 % for most of the life, so the estimate is wide.
* `lithium` (Li-FeS2): the voltage stays on a plateau, then collapses in a few days; the keyboard's percentage stays high until the end.
* `unknown`: no estimate is computed; alerts and display fall back on the keyboard's own percentage.
* The first two days after a detected battery change show **"new batteries, no estimate yet"**: the voltage of fresh cells relaxes during the first hours, no figure is drawn from it.

**Alerts** (`[alerts]`) are computed on the estimate when there is one, else on the keyboard's percentage; the notification says which ("Estimated charge 28%" or "Keyboard indication 28%"). With the default alkaline curve the thresholds 30 / 15 / 5 % fall at about 2460 / 2270 / 2060 mV, i.e. firmware 64 / 40 / 25 %. The keyboard's own low / critical alerts (Input `0x30`, labelled "keyboard alert") are independent of these thresholds and deduplicated with them: the keyboard is authoritative, one alert per event (#189).

### The keyboard's percentage steps down only at reconnections (#179)

[mesuré, correlation] `0x47` is fixed during a continuous session and steps down when the keyboard reconnects (99 → 98 → 96 on 2026-10-01, with no consumption behind it, VERIF-BATTERIE.md §1.2bis). The daemon **does not recompute or "repair" this value**. Instead every surface shows the **age of the last reading** (tray tooltip, window, widget "Last reading", `akmctl status` "Updated"), and a drop seen on the first reading after a reconnection raises no alert: the thresholds it crosses are disarmed silently; the next thresholds alert normally.

### History: real voltages (#180)

`history.jsonl` lines of schema 2 carry the real voltages in millivolts, `mv_0x46` and `mv_0x49`, and `voltage` (= `mv_0x46 / 1000`) for old readers. Lines written before schema 2 are marked `"voltage_valid":false` once at startup (kept, not deleted; a copy `history.jsonl.pre-schema2` is made once) and are ignored for the estimate, the replacement detection and the charts.

### Cost of reading the battery (#146) and Apple's schedule (#251)

Every read of the A1314's battery costs radio traffic, and UPower alone sends two GET_REPORT every 30 s (measured with `btmon`). The daemon follows Apple's own schedule (`akm-core/src/apple_model.rs`, [PARITE-APPLE.md](PARITE-APPLE.md)): first read 60 s after the connection, then every 4 h (1 h after a failure); a routine burst is `0x47`, GET Input `0x30`, `0x46`, `0x49`, 1 s apart, 3 s budget, stop at the first failure; one reader at a time (process mutex + `flock` shared with `akmctl dump`); circuit breaker after 3 consecutive silences, published in `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state` for the other emitters. A `Refresh()` (tray, D-Bus) is bounded to one per 5 min (#206). The estimate and the voltage history add no read. Optional, outside the daemon: `NoPollBatteries=true` in `UPower.conf` removes UPower's polling (`bluetooth/akm-conf.py upower`; `akmctl doctor` reports it).

### Signal (#174)

On the classic Bluetooth link (BR/EDR) the RSSI is **not in dBm**: it is the gap in dB to the controller's *Golden Receive Power Range* (Core Spec Vol 4 Part E §7.5.4); **0 = inside the ideal range**, negative = below, positive = above. The UI shows words first: "Signal: excellent (0)" for 0 or more, "good" for −1 to −5, "weak" below −5, with the raw value and no unit. JSON: `rssi_rel_db`, `rssi_quality`, `rssi_kind` (`bredr-golden-range`); `rssi_dbm` is kept as a **deprecated mirror** of `rssi_rel_db`. The D-Bus property `Rssi` (127 = unknown) carries the same relative value. `Signal: n/a` means the helper could not run: membership of the group `akm` is required.

## Other files

| File | Owner | Content |
|---|---|---|
| `~/.config/apple-kb-monitor/config.toml` | user | this document |
| `~/.config/apple-kb-monitor/keymap.toml` | user, written by `akmctl keymap` and the D-Bus `Keymap` interface | profiles of the manual key mapping: active profile, `models` (`05ac:0256`), preset (`apple`, `fkeys`, `linux-pc`, `none`), `hid_apple` parameters, `keys` (`F1..F12`, `Eject`, `Fn`… → `KEY_*`). Strict TOML subset, every error fatal. `akmctl keymap show`; installed as a udev hwdb by `akmctl keymap apply` (polkit). [TOUCHES.md](TOUCHES.md) §5 |
| `~/.config/apple-kb-monitor/selfcheck.env` | user, optional | environment of the 15-minute self-check: `AKM_SELFCHECK_ARGS=--gitea-issue` opens one Gitea issue per new grave problem (off by default, needs `~/.config/gitea/token`). [QA-AUTOMATIQUE.md](QA-AUTOMATIQUE.md) §3 |
| `/etc/apple-kb-monitor/hid-suspend.conf` | root, `backup=` | `enabled = true`: HID_CONTROL SUSPEND (`0x13`) before sleep and EXIT_SUSPEND (`0x14`) at wake, sent by the system units. `enabled = false` turns it off; the file must stay root-owned and not group/world writable, otherwise the feature is off. Alternative: `sudo systemctl mask apple-kb-monitor-suspend.service apple-kb-monitor-resume.service`. [VEILLE-HID.md](VEILLE-HID.md) |
| `/etc/modprobe.d/hid_apple.conf` | root, `backup=` | `options hid_apple fnmode=1`. Change it with `akmctl set fnmode N` (`--persist` writes the file through polkit) or `akmctl set param <name> <value>` (`fnmode` 0-4, `iso_layout` −1..1, `swap_opt_cmd` 0-2, `swap_ctrl_cmd`, `swap_fn_leftctrl`). Global to every Apple keyboard of the machine. |
| `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` | root, written by `akm-keymap-helper` only | the installed key mapping (previous file kept as `.akm-bak`; `akmctl keymap rollback`, `reset`). |
| `/etc/bluetooth/main.conf`, `/etc/UPower/UPower.conf` | root, optional | `FastConnectable = true`, `[Policy] Reconnect*`, `NoPollBatteries = true`: `bluetooth/akm-conf.py {bluez,upower} [--apply]`. [RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) §5.1 |
| `/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules` | package | `uaccess` on the 17 Bluetooth product ids; override with a file of the same name in `/etc/udev/rules.d/`. |

Environment variables: `APPLE_KB_RSSI_HELPER` (development override of `/usr/lib/apple-kb-monitor/rssi-helper`; not a privilege boundary, the helper carries the capability), `RUST_LOG` (`info` in the unit).

## Files written by the daemon and the CLI

| Path | Content |
|---|---|
| `~/.local/state/apple-kb-monitor/history.jsonl` | battery history (`ts`, `pct`, `schema`, plus `mv_0x46` / `mv_0x49` when the voltages were read); invalid points are not written; `akmctl history import FILE` merges an older file without duplicates |
| `~/.local/state/apple-kb-monitor/selfcheck.json` | last result of `akmctl selftest` |
| `~/.local/state/apple-kb-monitor/devname-backup-<UTC>.json`, `forget-backup-<UTC>.json` | `0600`, never overwritten: name stored in the keyboard before a write; host-side pairing data (no link key) before a clean forget |
| `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state`, `hid.lock`, `keymap.hwdb`, `ui-heartbeat.json` | volatile: published circuit breaker, cross-process HID lock, hwdb staged for the helper, window heartbeat read by the self-check |

The daemon runs with `UMask=0077` and `StateDirectory=apple-kb-monitor`: everything it writes is private to the user.
