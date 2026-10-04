# Reconnection, pairing and sleep of the Bluetooth keyboard

Topics: AX201 autosuspend, link recovery, system sleep, UPower polling, tools (`akmctl doctor` / `akmctl repair`).

Hardware: Apple A1314 ISO keyboard "alice's keyboard #1" `AA:BB:CC:DD:EE:F1` (BR/EDR, *legacy* PIN-code
pairing, without SSP), Intel AX201 USB adapter `8087:0026` (firmware `ibt-1040-4150`), BlueZ 5.87,
kernel 7.1.13-2-MANJARO, test PC.

Labels: **[measured]** observed on this machine (command or capture cited) · **[source]** read in the
BlueZ 5.87 or kernel code · **[hypothesis]** plausible, not yet proven here.

## 1. Symptom

The keyboard loses the link; "after a while" `bluetoothctl connect` fails
(`org.bluez.Error.Failed br-connection-create-socket`) and only the *forget + re-pair* sequence
brings it back. Last episode: cold restart of the PC on 2026-10-01 at 11:02, keyboard absent from 11:06 to 11:32,
"Forget" in the Plasma Bluetooth manager at 11:32:35, pairing wizard at 11:32:52, keyboard
back at 11:33:00 **[measured]** (`journalctl -b -1`, `-b 0`).

## 2. What the errors really say

| Message | Meaning | Label |
|---|---|---|
| `control_connect_cb() connect to …: Host is down (112)` | `EHOSTDOWN` on the HID control channel (PSM 17): **radio page timeout**, the keyboard did not answer the host's page. It is not a key refusal. | [source] `profiles/input/device.c`, `src/error.c` |
| `br-connection-create-socket` | BlueZ translation of `EIO`, the code returned by the HID profile when `control_connect_cb` fails. It is **the same** page timeout, seen from D-Bus. | [source] `btd_error_bredr_str()` |
| Key refused (which was **never** observed) | would appear as `br-connection-key-missing` (`EBADE`), `org.bluez.Error.AuthenticationFailed/Rejected`, or the signal `Device1.Disconnected("org.bluez.Reason.Authentication")` | [source] `src/error.h`, `src/device.c:device_disconnected()` |
| `HIDP GET_REPORT request timed out` | A *Feature Report* read (ioctl `HIDIOCGFEATURE` → uhid → bluetoothd) stayed unanswered; bluetoothd returns `EIO` to the caller, **without** dropping the link. | [source] `hidp_report_req_timeout()` |

Consequence: a failing `bluetoothctl connect` does not prove that pairing is broken; it proves that the
keyboard **was not listening** (it is asleep, or it is itself paging a host that does not answer it).

## 3. Measurements (2026-10-01, after the 11:33 re-pairing)

### 3.1 Pairing and link key

* `Paired/Bonded/Trusted = yes`, `LegacyPairing = yes`, `WakeAllowed = yes` **[measured]** (`bluetoothctl info`).
* Kernel key (`/sys/kernel/debug/bluetooth/hci0/link_keys`) and stored key
  (`/var/lib/bluetooth/AA:BB:CC:DD:EE:F2/AA:BB:CC:DD:EE:F1/info`): **identical SHA-256 fingerprints**
  (`b4e4395e8b4b…`), type 0 (legacy *combination key*) **[measured]**. No desynchronization at the time of
  the measurement.
* Gap: `pin_len` = 16 on the kernel side, `PINLength=0` in the file **[measured]**. After a restart the
  kernel will reload `pin_len = 0`. No effect for a keyboard: BlueZ requires security level MEDIUM
  (`BT_IO_SEC_MEDIUM`), for which a *combination key* is enough whatever `pin_len`; only level
  HIGH rejects a key from a PIN < 16 **[source]** `hci_conn_security()`, `hci_link_key_request_evt()`.
* The keyboard is in the kernel's accept list (`device_list`), so the host does *page scan*
  when it is disconnected **[measured]**; BlueZ puts it back there at each startup (`load_devices()` →
  `btd_device_set_temporary(false)` → `adapter_accept_list_add()`) **[source]**.
* On 27/09 at 12:59 and 13:03, bluetoothd could not rewrite the keyboard's `info` file
  (`g_rename() failed: No space left on device`) **[measured]** (`journalctl -b -2`). The write is atomic
  (temporary file + rename): the old file stays intact. But during a full disk, a
  **new** key (re-pairing, key change) would live only in memory and would be lost at the
  next restart → key refused by the keyboard → mandatory re-pairing **[hypothesis, mechanism
  [source] `store_link_key()`]**. `akmctl doctor` now reports these lines.

### 3.2 SDP record of the keyboard

`HIDReconnectInitiate = true`, `HIDNormallyConnectable = true`, `HIDRemoteWake = true`,
`HIDSupervisionTimeout = 0x1F40` (5 s), `HIDVirtualCable = false`, `HIDBatteryPower = true` **[measured]**
(decoded BlueZ SDP cache). BlueZ derives the `any` mode from it: the keyboard reconnects by itself (key press) and
the host can also page it when it is awake **[source]** `hid_reconnection_mode()`.

### 3.3 Host-side reconnection policy

* BlueZ HID profile: after a link loss, **6 attempts 30 s apart, then final give-up**
  ("at most 3 minutes") **[source]** `input_device_auto_reconnect()`; observed: `Host is down` at 04:01:08,
  :38, 04:02:08, :38, 04:03:08, :38 then nothing more **[measured]**.
* *policy* plugin (`[Policy] ReconnectAttempts/Intervals`): only applies after a disconnection by
  *timeout*. Until 01/10 11:16, these keys were under `[AdvMon]` and ignored
  (`Unknown key ReconnectUUIDs for group AdvMon`) **[measured]**; fixed at 11:16 (backup
  `main.conf.bak-20261001111627`).
* Nobody retried afterwards: the daemon had no recovery logic.

### 3.4 AX201 adapter and USB autosuspend

* `btusb enable_autosuspend = Y`, `power/control = auto`, `autosuspend_delay_ms = 2000` **[measured]**. It is
  systemd that imposes it for this exact identifier: `60-autosuspend-chromiumos.hwdb` contains
  `usb:v8087p0026*` and `60-autosuspend.rules` writes `power/control=auto` **[measured]**.
* As long as the keyboard is connected the adapter stays `active` (sampled 8 × 2 s). But
  `runtime_suspended_time = 430 343 ms`: the adapter spent **430 s suspended between startup (11:25)
  and re-pairing (11:33)**, that is precisely while the keyboard was supposed to page it **[measured]**.
  In this state the keyboard's *Connection Request* only reaches the host through the controller's USB
  *remote wake-up*.

### 3.5 Effect of our own reads and of UPower on keyboard sleep

Passive btmon capture (read-only, 11:37 → 12:27; raw capture deleted because it contains keystrokes) **[measured]**:

| Issuer | Radio requests | Cadence |
|---|---|---|
| UPower (read of `capacity` + `status` of the HID `power_supply` → `GET_REPORT 0x47`) | 2 | every 30-31 s, synchronous with UPower's `updated` field |
| `apple-kb-monitord` (`acquire`) | 16 (`0x47 ×3, 0xEA, 0xF5, 0x5A, 0x4F, 0xFF, 0x51-0x53, 0x46, 0x49, 0xF4, 0x4C, 0x09`) | every 15 min |
| Other reverse-engineering tools (sweep 0x00-0xFF) | 400+ in 4 s, 218 `HANDSHAKE ERR_INVALID_REPORT_ID` | one-off |

* Each request takes the link out of *sniff* mode (12.5 ms interval) and back again: 22 cycles
  `Exit Sniff Mode` → `Mode Change` in 10 min.
* Our daemon accounts for ≈ 16 requests / 15 min against ≈ 60 for UPower (≈ 21 %).
* Observed effect: the link never drops by itself — 7 continuous hours on the night of 30/09 to 01/10
  (04:13 → 11:02) and 2 days from 27 to 29/09 **[measured]** (`journalctl -b -2`, creation of uhid nodes).
  The keyboard therefore does not fall asleep in normal use; it only ends up "asleep" when **the host**
  disappears (restart, shutdown) or when its batteries are changed.
* The 11 `GET_REPORT request timed out` of 29/09 (15:09-15:21) and of 01/10 (04:00) precede or
  accompany disconnections, during reverse-engineering sessions **[measured]**. Cause-and-effect link
  not established **[hypothesis]**.

### 3.6 Link loss captured live (2026-10-01 12:13)

During the passive btmon capture, the link went down **[measured]** (sanitized extract without any keystroke):

| Time | Frame | Fact |
|---|---|---|
| 12:11:27-28 | — | a reverse-engineering tool reads **input** reports `0xFC`-`0xFF` in bursts (`GET_REPORT` type Input, 4 to 14 times each) |
| 12:12:58 → 12:13:08 | — | RE sampler: ~25 `GET_REPORT Feature` at 0.4 s intervals (`0x46`, `0xFF`, `0x49` … `0xF6`, `0xF7`); each request: *Exit Sniff* → *Mode Change Active* (~35 ms) → answer `a3 …` → back to sniff |
| 12:13:08.797 | #31 532 | `GET_REPORT Feature 0xFE` sent; acknowledged at baseband (*Number of Completed Packets* at .805) |
| 12:13:08.799 | #31 534 | *Exit Sniff Mode* accepted by the controller… but **no *Mode Change*, no answer, no more frames from the keyboard** |
| 12:13:12 / 12:13:25 | — | `HIDP GET_REPORT request timed out` (including the UPower `0x47` read at 12:13:21) |
| 12:13:28.823 | #31 538 | `Disconnect Complete`, reason **Connection Timeout (0x08)**: supervision timeout (20 s after the last frame), not a clean disconnection |
| 12:13:30 → 12:16:34 | — | BlueZ (*policy* plugin then HID profile) pages 9 times: `Page Timeout` every time; then nothing more (the maintainer notes count 7 up to 12:15:34, end of its copy of the capture) |
| ≥ 12:16 | — | adapter back in USB autosuspend (`usb_watch2.log`) |

Reading: the keyboard went **radio-silent all at once**, in the middle of a burst of reads, and no longer does
*page scan* afterwards. **[hypothesis]** A keyboard falling asleep normally would send an *LMP detach* (reason
*Remote*) rather than vanish through a supervision timeout (A1314 sleep behaviour never captured). Same signature at 04:00:13 the night before (loss "in the
middle of a burst" of the sampler, `docs/HARDWARE-HID-REPORTS.md` §5), return only 13 min
later, and on 29/09 15:09-15:21 (GET_REPORT timeouts then disconnections during reverse engineering)
**[measured]**. **[strong hypothesis]** the keyboard firmware locks up (or restarts) under certain
bursts of `GET_REPORT` on undocumented reports; it only recovers after a key press… or a
power off/on. The same read of `0xFE` at 11:47:23 (full sweep) had broken nothing: it
is not a deterministic "killer" report. *Counter-audit*: the maintainer notes on the contrary retain the narrow
form "a GET `0xFE` can freeze the firmware" (2 lock-ups out of ~27 reads, always right after `0xF7`).
Both readings are compatible (non-deterministic trigger); **not settled** without the E8 / §0.2 protocol.

### 3.7 System sleep

No `PM: suspend entry` line in the whole retained log (27/09 → 01/10) **[measured]**: PC sleep
is not the origin of the observed episodes. It is nonetheless handled (§6).

## 4. Root cause

**What is proven**:

1. **It is not the pairing.** No trace of authentication failure in any incident; kernel key =
   stored key; `br-connection-create-socket` and `Host is down` = page timeout, not a refused key
   (§2, §3.1).
2. **The link drops through radio silence of the keyboard, not through clean sleep**: supervision timeout
   (reason 0x08) in the middle of a burst of `GET_REPORT`, captured frame by frame at 12:13 (§3.6), same
   signature at 04:00 and on 29/09. The bursts come from the reverse-engineering tools run on this
   machine in recent days; background traffic (UPower 2 req./30 s, daemon 16 req./15 min) multiplies the
   opportunities (§3.5).
3. **Then the keyboard no longer listens** (page timeout on every page) and **BlueZ gives up** after
   2 to 3 min (§3.3); nobody retried.
4. **Reconnection by key press did not succeed** in the logged episodes (11:06-11:32: no uhid
   node or bluetoothd line) **[measured]**; the adapter was then in USB autosuspend (§3.4).
5. **Re-pairing "repairs" for a reason other than the key**: it requires switching the
   keyboard off/on (pairing mode), which resets a locked-up firmware, then it is the **host** that pages a
   keyboard that listens. **[strong hypothesis, consistent with 1-4]**: a simple off/on, without
   unpairing, is enough.

**Root cause retained**: lock-up (or restart) of the keyboard firmware under bursts of HID reads,
aggravated on the host side by BlueZ giving up after 3 min and by an adapter in USB autosuspend when
the keyboard must reconnect; re-pairing was only the manipulation that happened to include a
keyboard restart.

Hypotheses ruled out: desynchronized key (§3.1), keyboard missing from the accept list (§3.1),
`pin_len` (§3.1), system sleep (§3.7), SET_REPORT write (none in the code, verified by search).

Still to confirm on the hardware (§7): (a) after a loss of this kind, is a **key press** enough, or
must the keyboard be **switched off/on**? (b) with the adapter **without** autosuspend, does the keyboard's *Connect Request*
arrive?

## 5. Fixes

### 5.1 System configuration (repository files, applied by the administrator)

| File | Effect | Why |
|---|---|---|
| `udev/61-akm-bt-adapter-no-autosuspend.rules` | `power/control=on` for **this adapter only** `8087:0026`; must come after `60-autosuspend.rules` | removes link 3 (§3.4); cost ≈ 0.1 W on a desktop PC |
| `akmctl doctor --fix` | `[General] FastConnectable = true`; `Reconnect*` guaranteed in `[Policy]` | interlaced *page scan*: the host answers the keyboard's short page train faster; safeguard against the keys returning to the wrong section |
| `akmctl doctor --fix --optional` (optional) | `NoPollBatteries = true` | removes the 2 GET_REPORT / 30 s (§3.5); the keyboard can finally fall asleep and save its batteries. The daemon remains the battery source (BlueZ BatteryProvider, read every 15 min) |

Not retained: `options btusb enable_autosuspend=0` (disables autosuspend for **all** btusb
devices and requires reloading the module); `Experimental = false` (unrelated to BR/EDR);
`IdleTimeout` in `input.conf` (leave at 0: BlueZ must not drop an idle keyboard).

### 5.2 `apple-kb-monitord` daemon

* **Safe HID read policy** (`akm_core::read_policy`): the daemon now only requests
  `0x47` (percentage, declared report), `0x46` (voltage mV) and `0x49` (filtered voltage) — 3 requests instead
  of 14, no more `0xEA` probe, never `0xFE` nor any undeclared report, never a sweep;
  **only if a key was pressed in the last minute** (keyboard awake, link active; only the
  timestamp of the last keystroke is kept, never its content); a single reader (mutex + `flock`
  on `$XDG_RUNTIME_DIR/apple-kb-monitor/hid.lock`, to be used by RE tools too); 1 s between two
  requests, 2 s budget, stop at the first error. Idle keyboard: no request, the percentage comes from the
  kernel.
* Reconciliation with BlueZ: the link guardian pushes to the acquisition machine the set of
  actually connected keyboards (at startup, when bluetoothd comes back, on wake, every 5 min); the
  "BlueZ absent" probe is capped at 12 attempts.
* `akm_core::recovery` — pure state machine, tested in simulated time:
  `connected` · `dormant` (keyboard asleep or link lost recently, no alert) · `unreachable`
  (after link loss / wake / startup: no answer for 10 min despite ≥ 3 pages; **one**
  notification per episode) · `auth-failed` (reason `Authentication`, `key-missing`, pairing removed:
  pages **stopped**, notification "re-pairing required") · `suspended`.
* `Device1.Connect` calls: never less than 20 s apart; after a link loss 20 s, 40 s,
  1 min, 2 min then every 5 min; after one hour every 15 min; keyboard asleep: one call
  every 5 min then 15 min (catches a missed key-press reconnection while the keyboard is awake);
  "busy" answer from BlueZ: new attempt 30 s later, not counted as a failure.
* Sleep (logind): `delay` inhibitor; on `PrepareForSleep(true)` no more HID reads or calls, wait
  for an ongoing read to finish (≤ 3 s) then release; on wake a 4 s grace period then fast
  resumption.
* State published on the session bus: object `/com/agenceapi/AppleKbMonitor1/Link`, interface
  `com.agenceapi.AppleKbMonitor1.Link`, methods `Status()` (JSON: `mac`, `name`, `health`, `since`,
  `attempts`, `failures`, `last_error`, `last_reason`, `updated`) and `Reconnect()` (subject to the same
  20 s minimum). Verified for real on 01/10 (test instance): `health = connected`, logind
  `delay` inhibitor active.

### 5.3 Tools

* `akmctl doctor` — one command: BlueZ state, key fingerprint (never the key), accept list,
  adapter autosuspend, `main.conf`, UPower, classified bluetoothd logs, hidraw, daemon health,
  verdict and advised action.
* `akmctl repair` — first without breaking anything: key press + spaced pages, then **switch the
  keyboard off/on** + pages; re-pairing only afterwards, and only after typing the confirmation word `OUBLIER` in a
  terminal; refuses if the link is healthy. Widget right-click menu: "Repair the link…".

### 5.4 Clean forget of a connected keyboard, like macOS

When `akmctl repair` must remove the pairing while the keyboard is **still connected** (`--force` on a live
link, or key refusal observed during a connection), it reproduces what macOS 26.5 does on "Forget":

| Step | What is done | Evidence |
|---|---|---|
| 1 | pre-flight: keyboard connected, health `connected` in `akmctl doctor` and no failure, circuit breaker closed, stdin and stdout are a terminal | — |
| 2 | **backup** on the machine: `~/.local/state/apple-kb-monitor/forget-backup-<YYYYMMDDTHHMMSSZ>.json` (0600, never overwritten): MAC, name, alias, `Paired`/`Bonded`/`Trusted`, adapter path and address, device path. **No link key** (never read) | — |
| 3 | explanation (what is going to happen, effect of `0x41` not measured, how to re-pair) then typing `OUBLIER`; never non-interactively | — |
| 4 | **one** SET Feature `0x41` `RecantConnection`, ID only, wire `53 41` (`Forget` operation of the registry, once per session, one-byte gate, bytes logged) | [decompiled + listing] `bluetoothd` `FUN_1005a41e4` (maintainer notes) |
| 5 | the daemon suppresses the disconnect notification (`ExpectDisconnect()`, 15 s; equivalent of `SuppressDisconnectNotifications`) | [disassembly] `-[AppleBluetoothHIDDevice recantConnection]` |
| 6 | wait **2000 ms** for the link to drop; if it does not drop, continue as Apple does ("timed out waiting to recant") | [decompiled] `FUN_1005a3f64` |
| 7 | **only then** `org.bluez.Adapter1.RemoveDevice` | [decompiled] `FUN_1005a3f64` ("will unpair") |
| 8 | existing pairing wizard, wait for the keyboard paired + connected, `Trusted`, then final check `akmctl doctor` | — |

* If `0x41` is not sent (hidraw gate unavailable, lock taken) or not accepted (error, silent keyboard): **nothing
  is removed**, back to the "wake + reconnect" step, no new attempt.
* **Circuit breaker open** (Apple rule R3: three consecutive silences, no more emission until a new
  connection or a sleep): the pre-flight learns it from the daemon (D-Bus, otherwise the published state
  `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state`); `0x41` is **not** written (the keyboard is silent, the order would not
  be heard), the existing "wake + reconnect" step follows, then the user is informed (the circuit breaker
  closes again on reconnection; rerun `akmctl repair` if forgetting is still needed). The write gate itself
  (`hidraw::WriteDoor`) applies the same verdict: an `akmctl` started while the daemon has opened the circuit breaker writes
  neither `0x41` nor `0x55`.
* Stop at the first failure; each byte (`[hid-write] Forget …`) and each decision (`[forget] …`) are logged on stderr.
* A keyboard that is **not connected** is unpaired as before, without `0x41`: macOS only sends `RecantConnection` to a
  connected device.
* `0x41` is only reachable through `akmctl repair` (a test refuses any other reference to the `Forget` operation or to
  `forget::run`): neither D-Bus nor the widget menu (which only opens `akmctl repair` in a terminal, where `OUBLIER`
  must be typed). No test writes to the keyboard or calls `RemoveDevice`: simulator and spy only.
* **Actual effect on the keyboard: not measured** (mere disconnection or forgetting this host, maintainer notes).
  `0x44` `FullFactoryDefault` (Lion, erases all keys) remains forbidden.

## 6. Keyboard and PC sleep: expected behaviour after the fixes

| Situation | Before | After |
|---|---|---|
| Daemon reads | 14 reports / 15 min, keyboard active or not | 3 reports, only after a recent keystroke |
| Idle keyboard | never asleep (UPower 2 req./30 s) | falls asleep if `NoPollBatteries`; `dormant`, comes back on key press; safety net: a page every 5 then 15 min |
| PC asleep | HID reads possible during the shutdown, passive wait on wake | reads suspended before the shutdown; on wake pages at +4 s, +24 s, +64 s… |
| PC restart | BlueZ gives up after 3 min; re-pairing | adapter never suspended; spaced pages with no time limit; clear alert at 10 min |
| Key actually refused | indistinguishable from a sleeping keyboard | `auth-failed`, notification, `akmctl repair` |
| Keyboard silent after a burst of reads | re-pairing | notification "press a key, otherwise switch it off/on"; pairing is kept |

## 7. Controlled test on the hardware (to be done with the author)

Prerequisites: the author is at the keyboard, another input method is available, **no
reverse-engineering tool is running**.

First test, without any command (after a drop like the one at 12:13): press a key;
if nothing within 10 s, switch the keyboard off/on **without forgetting it** in Plasma. If it comes back, the pairing
was not at fault (§4, item 5).

```sh
tests/live/reconnect_probe.sh            # passive btmon + USB state + log, then:
#   1. bluetoothctl disconnect AA:BB:CC:DD:EE:F1   (only once)
#   2. the author waits 60 s then presses ONE key
#   3. the script waits 120 s for the reconnection and classifies what it sees
```

Reading the result: keyboard `Connect Request` received and accepted → reconnection by key press
works in this state; no `Connect Request` while the adapter was `suspended` → role of
autosuspend confirmed; `Link Key Request Negative Reply` / `Authentication Failure` → key at fault (then
`akmctl repair`).

## 8. Remaining risks

* The keyboard firmware lock-up is deduced from three consistent drops, not demonstrated by a
  deliberate reproduction (which would require hammering the keyboard: excluded). The safe read policy
  removes the cause on the daemon side; UPower (2 `0x47` reads / 30 s through the kernel) remains as long as
  `NoPollBatteries` is not applied.
* Reconnection by key press with the adapter in autosuspend: not captured until the §7 test has
  been done; the udev rule neutralizes this path in any case.
* A keyboard that pairs with **another host** (it only remembers one) will behave exactly like a refused
  key: only `akmctl repair` brings it back.
* Reverse-engineering sessions that sweep Report IDs (including input reports, 12:11:27)
  are the most likely cause of the recent drops: only to be run under the `hid.lock` lock, never
  during normal use of the keyboard.
* The author "forgot" then re-paired the keyboard at 11:32 and forgot it again at 12:24 (Plasma):
  each re-pairing creates a new key; none was fixing a wrong key.
* `~/.config/apple-kb-monitor/config.toml` contains the `mqtt-bridge` configuration (warnings
  `unknown key [mqtt]` at startup): no effect on the link, to be separated.
