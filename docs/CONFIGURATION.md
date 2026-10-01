# Configuration

The Rust daemon `apple-kb-monitord` reads one optional file, `$XDG_CONFIG_HOME/apple-kb-monitor/config.toml` (default `~/.config/apple-kb-monitor/config.toml`). A missing file means the defaults; an unknown key or a bad value is logged as a warning and ignored. (The former display/DDC/MQTT `config.toml` no longer exists.)

```toml
[battery]
chemistry = "alkaline"     # "alkaline" | "nimh" | "lithium" | "unknown"   (default: alkaline)

[alerts]
thresholds = [30, 15, 5]   # % of the ESTIMATED charge (see below); one notification per threshold crossed
hysteresis = 3             # a threshold is re-armed at threshold + 3 points
critical = 5               # thresholds <= 5 are sent as "critical"
enabled = true

[notifications]
connection = true          # "disconnected" / "reconnected (N %)", low urgency
battery_replaced = true    # "new batteries detected"
defer_to_powerdevil = true # KDE PowerDevil already warns about this keyboard: one distinct reminder only (#254)

[display]
apple_percent = true       # also show the percentage "as macOS shows it", labelled "Apple display" (#213)

[apple]
will_shutdown = true       # tell the keyboard once at shutdown / restart, as macOS does (#191)   (default: true)
```

`will_shutdown` (default **true**, because it is what macOS sends to this keyboard at every shutdown and restart): when the computer shuts down or restarts, the daemon sends **one** Feature report to the keyboard, `0x40` (`WillShutdown`), made of the report id alone (on the Bluetooth link `53 40`), exactly as Apple's driver does. Conditions, all required: the option is on, the keyboard is connected, nothing was sent yet in this run, the circuit breaker is closed, at least 1 s since the previous hardware access. A failure is logged and never retried, and never delays the shutdown by more than 3.5 s. It is the **only** write this software can make to the keyboard (`docs/PARITE-APPLE.md`). Set `will_shutdown = false` to send nothing; `akmctl status` shows the current choice (`Shutdown:` line, `will_shutdown` key of `--json`). Two doors lead to the same single write: the logind `PrepareForShutdown` delay inhibitor held by the daemon, and the user unit `apple-kb-monitor-shutdown.service` (`ExecStop=akmctl shutdown-notify --only-if-stopping`); `systemctl --user mask apple-kb-monitor-shutdown.service` disables the unit, the inhibitor stays governed by the option.

`defer_to_powerdevil` (default **true**, #254): see `docs/INTEGRATION-KDE.md` §3. With `false` the 30 / 15 / 5 % alerts are sent as usual even though PowerDevil also warns.

`apple_percent` only adds a secondary figure (tray tooltip, window, widget, `akmctl status`): IOBluetooth remaps the raw `0x47` value (54..100 -> 100 %, 21..53 -> 21 + (raw - 21) x 2.4375, below 21 unchanged). Alerts and the estimate do not use it.

## Battery: what is shown and what the alerts use

The keyboard reports a percentage (report `0x47`, = kernel `power_supply`). It is **not a remaining charge**: the firmware interpolates the pair voltage (`0x49`) on a factory table (2954 / 2506 / 2404 / 2054 mV for 100 / 75 / 50 / 25 %). On alkaline cells that scale is optimistic in the middle of the battery's life: firmware 75 % is about 35 % of real charge, 50 % about 25 %, 30 % about 7 % (`docs/VERIF-BATTERIE.md` §2). It is shown as **"keyboard indication"** and never rewritten.

Next to it the daemon shows an **estimate** (#178): the voltage `0x49` (else `0x46`) projected on a discharge curve of the chemistry declared in `[battery] chemistry`, with a range (+-10 points for alkaline, +-15 for NiMH and lithium). The curves are a **[hypothèse]**: read by eye from the Energizer E91 and L91 datasheets and from Eneloop-type NiMH curves at a low drain (~1 mA, 21 degC), +-0.03 V per cell, plus about +-50 mV per pair of ADC calibration. Pair voltage to real charge, alkaline, indicative:

| Pair (mV) | 2950 | 2775 | 2600 | 2506 | 2404 | 2200 | 2124 | 2054 |
|---|---|---|---|---|---|---|---|---|
| Firmware % | 99 | 90 | 80 | 75 | 50 | 35 | 30 | 25 |
| Alkaline estimate | ~84 | ~62 | ~44 | ~35 | ~25 | ~10 | ~7 | ~5 |

* `alkaline` (default; also zinc-carbon): the only chemistry consistent with the 2.97-2.99 V measured a few hours after a battery change.
* `nimh` (Eneloop type): a flat plateau at 1.20-1.25 V per cell; the keyboard's own percentage stays at 50-75 % for most of the life, so the estimate is wide.
* `lithium` (Li-FeS2): the voltage stays on a plateau, then collapses in a few days; the keyboard's percentage stays high until the end.
* `unknown`: no estimate is computed; alerts and display fall back on the keyboard's own percentage.
* The first two days after a detected battery change show **"new batteries, no estimate yet"**: the voltage of fresh cells relaxes during the first hours, no figure is drawn from it.

**Alerts** (`[alerts]`) are computed on the estimate when there is one, else on the keyboard's percentage; the notification says which ("Estimated charge 28%" or "Keyboard indication 28%"). With the default alkaline curve the thresholds 30 / 15 / 5 % fall at about 2460 / 2270 / 2060 mV, i.e. firmware 64 / 40 / 25 %, instead of firmware 30 / 15 / 5 % (which is about 7 % real charge for the first one).

### The keyboard's percentage steps down only at reconnections (#179)

[mesuré, correlation] `0x47` is fixed during a continuous session and steps down when the keyboard reconnects (99 -> 98 -> 96 on 2026-10-01, with no consumption behind it, `docs/VERIF-BATTERIE.md` §1.2bis). The daemon **does not recompute or "repair" this value**. Instead:

* every surface shows the **age of the last reading** (tray tooltip "Updated N min ago", window "Updated", widget "Last reading", `akmctl status` "Updated");
* a **drop seen on the first reading after a reconnection raises no alert**: the thresholds it crosses are disarmed silently (a link artefact is not a discharge); the next thresholds alert normally.

### History: real voltages (#180)

`history.jsonl` lines of schema 2 carry the real voltages in millivolts, `mv_0x46` and `mv_0x49`, and `voltage` (= `mv_0x46 / 1000`) for old readers. Before, `voltage` was the constant `0xF5 x 3.3 / 1023` (2.98 / 2.90 V on every line, 2 136 lines on the author's machine): at startup the daemon marks every line without `schema` as `"voltage_valid":false` (the value is kept, not deleted; a copy `history.jsonl.pre-schema2` is made once). Such lines keep their `pct` but are **ignored** for the autonomy estimate, the replacement detection and the charts.

### Cost of reading the battery (#146)

Reading the A1314's battery costs radio traffic: the kernel (`hid-input`) sends a `GET_REPORT 0x47` at every read of `capacity`/`status` because the keyboard never publishes its battery by itself, and **UPower does that every 30 s** (2 requests per cycle, measured with `btmon`: the keyboard never goes back to sniff mode). The daemon is the small part (about 16 requests per 15 min, about 21 % of the traffic) and obeys the safe read policy of `akm-core/src/read_policy.rs` (`docs/RECONNEXION-PAIRAGE.md` §3.6): only `0x47`, `0x46`, `0x49`; only while a key was pressed in the last 60 s; one reader at a time (process mutex + `flock`); 1 s minimum between requests (as macOS 26.5), 2 s budget, stop at the first failure, circuit breaker after 3 failures in a row (#214); a full read every 4 h at most (a `Refresh()` is bounded to one per 5 min, #206). **The estimate and the voltage history add no read**: they reuse the `0x46` and `0x49` values already read by that policy. Optional, outside the daemon: `NoPollBatteries=true` in `UPower.conf` removes UPower's polling; the daemon then stays the source of the battery (BlueZ `BatteryProvider`). See `bluetooth/akm-conf.py upower`.

### Signal (#174)

On the classic Bluetooth link (BR/EDR) the RSSI is **not in dBm**: it is the gap in dB to the controller's *Golden Receive Power Range* (Core Spec Vol 4 Part E §7.5.4); **0 = inside the ideal range**, negative = below, positive = above (legal). The UI shows words first: "Signal: excellent (0)" for 0 or more, "good" for -1 to -5, "weak" below -5, with the raw value in parentheses and no unit. JSON (`akmctl status --json`, D-Bus `Json`): `radio.rssi_rel_db`, `radio.rssi_quality` (`excellent`/`good`/`weak`), `radio.rssi_kind` (`bredr-golden-range`); the former `radio.rssi_dbm` / `rssi_dbm` is kept as a **deprecated mirror** of `rssi_rel_db` (same value, misleading name). The D-Bus property `Rssi` (127 = unknown) carries the same relative value.

## What is configurable

| Item | Where | Notes |
|---|---|---|
| RSSI helper location | env `APPLE_KB_RSSI_HELPER` | Development/test override of `/usr/lib/apple-kb-monitor/rssi-helper` (`rssi.rs`). Not a privilege boundary: the helper itself carries the capability. |
| Low-battery thresholds, schedule | `~/.config/apple-kb-monitor/config.toml` | Read by `apple-kb-monitord` (see `akm-core/src/config.rs`) |
| `fnmode` | `/etc/modprobe.d/hid_apple.conf` (`options hid_apple fnmode=1`) | Applied immediately by `post_install`. Change it with `akmctl set fnmode N --persist`. The Diag tab of `apihub-app` shows the applied value (`/sys/module/hid_apple/parameters/fnmode`) and the configured one (`/etc/modprobe.d`), and flags a difference |
| Service to enable | `systemctl --user enable --now apple-kb-monitord.service` | The only daemon (the Python CLI and its service were removed in 3.1.0-6) |
| Special keys | `/etc/keyd/apple-keyboard.conf` (kept on upgrade, `backup=`) | Device `05ac:0256` |
| uaccess rule | `/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules` | Override by placing a file of the same name in `/etc/udev/rules.d/` |

## Files written by the app

| Path | Content |
|---|---|
| `~/.local/state/apple-kb-monitor/history.jsonl` | Battery history (timestamp, percentage, `mv_0x46`, `mv_0x49`, `schema`); invalid points are not written. The pre-3.1 `~/.local/share/...` file is copied once |

The Python CLI keeps its own state under `$XDG_RUNTIME_DIR/apple-kb-monitor/`.
