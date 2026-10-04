# Renaming the keyboard

Two names exist. The alias only concerns this computer; the keyboard's own name is stored in the keyboard and seen by every device. Both are changed with an ordinary command.

| | (a) Alias on the computer side | (b) Own name, stored in the keyboard |
|---|---|---|
| Command | `akmctl rename <name>` / `--reset` | `akmctl rename --device-name <name>` (writes, after **one** confirmation), `--check`, `--dry-run`, `--show`, `--restore` |
| Where | BlueZ, `org.bluez.Device1.Alias`, persisted in `/var/lib/bluetooth/<adapter>/<MAC>/info` (`Alias=`) | keyboard firmware (BCM2042): read from `0x51-0x54` (4 × 8 B ASCII), written to `0x55` `LongDeviceName` (65 B: id + 64) |
| Visible to | this computer only (KDE Bluetooth, `bluetoothctl`, widget, `akmctl`) | any device that pairs with the keyboard |
| State | **implemented** | **tested on the hardware** on 2026-10-02 (firmware `0x0050`): write accepted, read back identical within the same connection |

## 1. (a) Alias: what is delivered

* `akmctl rename <name>` / `akmctl rename --reset` (`--mac` to target a keyboard). Exit codes: 0 OK, 1 error (name refused, BlueZ), 2 daemon missing.
* Session D-Bus: `com.agenceapi.AppleKbMonitor1.SetAlias(s mac, s name) -> s` on the root object, `Device.SetAlias(s name) -> s` on the keyboard object; `Name` property (alias, otherwise own name) on both. Empty name = back to the original name (BlueZ behaviour).
* Validation (`akm-core::alias`, shared by all clients): leading and trailing spaces removed, max 64 characters and 248 UTF-8 bytes (HCI limit), refusal of control characters (Cc), line/paragraph separators, invisible characters and bidirectional overrides (U+200B-200F, 2028-202E, 2060-2064, 2066-2069, FEFF). The daemon only writes to a BlueZ device whose `Modalias` is Apple.
* Widget right-click menu: "Rename keyboard…" (`kdialog` dialog, otherwise `zenity`). Plasma widget, State tab: "Rename", then "Apply" / "Keyboard's own name" / "Cancel". System Settings module, Name page: "Rename" / "Use the keyboard's own name". **These buttons only touch the alias.**
* Display: widget tooltip, `akmctl status [--json]` (`name`, `alias`), `apple-kb-monitord --json` (`name`, `keyboard.device.alias`), snapshot JSON, widget.
* A rename done elsewhere (`bluetoothctl`, KDE Settings) is picked up through `PropertiesChanged.Alias`.
* **Traceability (2026-10-02, alias reset twice with no cause found).** Inventory of everything that writes the alias in the tree: (1) `apple_kb_monitord::alias::rename`, the only writer, called by `SetAlias` (root and `Device`, hence `akmctl rename`, the KCM `NamePage`/`Store.qml`, the widget `DaemonLink.qml`), by the menu's "Rename keyboard…"; (2) **indirectly**, `Adapter1.RemoveDevice` (`akmctl repair`, the `forget` command, Plasma's "Forget", `bluetoothctl remove`): BlueZ erases the device directory, alias included, and a new pairing takes back the own name; the alias is copied into the `forget-*.json` backup but **never restored**. No test, no `selftest`, no `doctor` writes the alias: tests go through fake engines (`FakeAlias`, `Spy`) or a private bus (`dbus-run-session`, `tests/e2e`). From now on every write is logged with its caller (D-Bus name, pid, program read from `/proc/<pid>/comm`, or "tray"), the resulting alias is remembered in `~/.local/state/apple-kb-monitor/alias.json` (real BlueZ engine only), the daemon logs as a warning any change coming from elsewhere (`BlueZ alias of … changed OUTSIDE this monitor`, with the expected alias, who set it and when) and `akmctl selftest` reports as **info** an alias different from the remembered one.
* Limit: the kernel's `HID_NAME` (`/sys/.../uevent`) keeps the old name until the next reconnection; the application shows the alias first.

## 2. (b) Own name: the command

```
akmctl rename --device-name "Office"
```

What you see (in the session language when one of the 19 shipped catalogs matches it, English otherwise; English shown here):

```
Name stored in the keyboard: “alice's keyboard #1” → “Office”
Pre-flight: ok (doctor green, battery 99 %, breaker closed, outgoing MTU 185 >= 66)
Backup of the current name: ~/.local/state/apple-kb-monitor/devname-backup-<timestamp>.json
Write “Office” into the keyboard's memory? [y/N] y
✓ Name written into the keyboard and read back identical: “Office”
  BlueZ may show the old name until a later connection; persistence across a battery change is not measured.
```

| Command | Effect | Writes? |
|---|---|---|
| `akmctl rename --device-name <name>` | pre-flight, backup, **one** confirmation `[y/N]` (`[o/N]` in French), one write, immediate read-back, verdict | yes |
| `… --yes` (`-y`) | the same without the question; works without a terminal (scripts, KDE module) | yes |
| `… --check` | the whole pre-flight and the backup; asks nothing | no |
| `… --dry-run` | the 65 bytes that would be sent, field by field; no `pkexec` | no |
| `… --verbose` | adds the details: frame, bytes on the wire, proof by disassembly, `[devname]` log | depends on the command |
| `akmctl rename --device-name --show` | name read from `0x51-0x54` and its bytes, from the daemon's cache | no |
| `akmctl rename --device-name --restore <backup.json>` | rewrites a backup, same sequence (one confirmation or `--yes`) | yes |

Without a terminal and without `--yes`, the command refuses **before** any pre-flight, any `pkexec` and any backup (code 10) and says to add `--yes`. `--write-device-name` (old option) is still accepted and does nothing. The `[apple] allow_device_name_write` key of `config.toml` is obsolete: read, ignored.

Exit codes: **0** written and read back identical (or `--check` green) · 1 error · 2 daemon missing · **10** no confirmation possible · **11** pre-flight refused · **12** cancelled (answer other than yes; nothing written) · **13** written but read-back impossible (check later with `--show`) · **14** read back different (the rollback command is shown) · **15** uncertain write (the write failed partway, with no retry: a frame may have gone out, check with `--show`) · 64 usage.

No password is asked in the active local session: the MTU read goes through the polkit action `com.agenceapi.AppleKbMonitor.hid-inspect` (`pkexec akm-helper hid-inspect`, read-only, `allow_active = yes`). `akm-helper hid-inspect` is a separate executable, without a verb, that cannot send anything: its polkit action is bound to its path and nothing else, while `akm-helper hid-control` (SUSPEND / EXIT_SUSPEND) stays at `auth_admin`.

### System Settings module

"Name" tab: the "**Write the name into the keyboard…**" button opens a confirmation dialog ("Write “X” into the keyboard's memory?"), then runs `akmctl rename --device-name=<name> --yes`; "**Check (nothing written)**" runs the same command with `--check`. The result is shown in the page. No terminal, no shell: D-Bus `Settings.RunAkmctl` (closed command list) with an argument array, the name as **a single argument**, 60 s deadline (`KCM.md`).

## 3. Measured facts (2026-10-02, real keyboard, firmware `0x0050`)

| # | Question | Answer |
|---|---|---|
| U5 | does the firmware accept a SET Feature `0x55`? | **[measured] yes**: the 65-byte write succeeds (HANDSHAKE SUCCESSFUL); the link did not move (circuit breaker at 0) |
| U4 | do `0x51-0x54` reflect `0x55`, and when? | **[measured] yes, within the same connection**, without turning the keyboard off and on: read back ~75 s after the write, the written name then `0x00` up to 32 bytes |
| U6 | outgoing MTU of the control channel | **[measured] 185** (≥ 66 required); re-read on every command, never assumed |
| — | BlueZ `Device1.Name` | **[measured]** keeps the old name in cache; it refreshes on a later connection |
| U3 | persistence after power-off or a battery change | **not measured**: Apple never rewrites the name on reconnection (so the keyboard is expected to remember it, [deduction]); the backup allows rewriting it |

The frame is the one of Lion 10.7.5 `-[AppleBluetoothHIDDevice setDeviceName:]`, established byte for byte by disassembly (U1, U2, U7, see §3): **one** 65-byte SET Feature `0x55`, `55` + the name in UTF-8 + `0x00` padding, i.e. `53 55 …` (66 bytes) on the wire; no other frame.

```
“alex”
report : 55 61 6c 65 78 00 × 60
wire   : 53 55 61 6c 65 78 00 × 60
```

Reference: `tests/fixtures/devname/lion_setdevicename_frames.json`; `devname::tests::frames_match_the_lion_fixture_byte_for_byte` and `confirmed_the_fixture_frame_is_sent_once_after_the_backup_then_read_back` check that the spy receives exactly these bytes, once, after the backup.

## 4. Sequence of a write (`akm-core::devname::run`)

Stop at the first failure, **never a retry**.

1. **Name validated** (`devname::validate`): printable ASCII (`0x20`-`0x7E`), 1 to 32 characters, nothing is trimmed, no leading or trailing space, no `\` (escaping in BlueZ's `info` file). 32 = what `0x51-0x54` can read back.
2. **Pre-flight**: keyboard connected; batteries ≥ 20 % (or "normal" state); circuit breaker closed; last reading complete; `akmctl doctor` green; outgoing MTU of the L2CAP control channel ≥ 66, read by the single `pkexec akm-helper hid-inspect --mac <MAC>` (`getsockopt`, read-only). Linux `hidp` does not fragment: a frame that is too long would cut the HID session.
3. **Backup** of the current name, before any write: `~/.local/state/apple-kb-monitor/devname-backup-<YYYYMMDDTHHMMSSZ>.json`, new file, **0600** (directory 0700).
4. **Single confirmation** `[y/N]` (or `--yes`).
5. Connection checked again; the hidraw node is opened under the HID lock shared with the daemon (`hidraw::WriteDoor`). The address of the opened node must be the one of the pre-flight and the backup (`--mac`, otherwise the keyboard followed by the daemon): with two Apple keyboards connected, if the other one's node opens, nothing is written.
6. **One** write through `WriteSession` and the registry: operation named `DeviceName`, id `0x55`, exactly 64 data bytes, once per session; 65-byte hardware door.
7. Wait for the spacing (1 s), then **immediate read-back** of `0x51-0x54` through the same door (`WriteDoor::read_name`), through `read_policy::SafeSource`: registry, 1 s between two requests, circuit breaker, stop at the first failure; each frame must be the id + 8 bytes.
8. **Comparison** with the first 32 bytes written, verdict (codes 0, 13, 14; 15 if the write itself failed after the node was opened).
9. The daemon is notified (D-Bus `RereadName`, which answers `(accepted, text)`): it immediately forgets its `0x51-0x54` fragments (it never shows a name it knows is stale) and re-reads those four reports alone, never the routine reading: right away, or, if a re-read happened less than 30 s ago, at the end of those 30 s (request deferred, never dropped). The time of the last keyboard access is shared between `akmctl` and the daemon (`hid.last` next to `hid.lock`): the 1 s gap also applies between `akmctl`'s read-back and the daemon's.

Unchanged protections: registry (`0x55` writable by `DeviceName` only), `WriteSession`, read policy and budgets, circuit breaker (the daemon's decides), MTU ≥ 66, backup before writing, never a retry. No D-Bus method writes into the keyboard.

## 5. Rollback

```
akmctl rename --device-name --restore ~/.local/state/apple-kb-monitor/devname-backup-<timestamp>.json --yes
```

The exact command is shown when the name read back differs (code 14). It checks the backup (4 × 8 bytes, a non-empty name then NUL, consistent with the readable name) and rewrites those bytes as they are: the backup keeps the 32 raw bytes even if they are not ASCII (macOS writes UTF-8, "Cécile's keyboard"); only a typed name must be ASCII. It refuses a backup of another keyboard than the target (MAC address), then follows the same sequence, the current name saved first. Without `--yes`, it asks the same `[y/N]` question. No rollback is automatic.

## 6. What remains to be known

* **U3, persistence**: not measured. To do (E3): compare `akmctl rename --device-name --show` before and after a battery change. If the name is lost, the command of §5 rewrites it.
* The name shown by BlueZ (`Device1.Name`, hence KDE and `bluetoothctl` when no alias is set) follows on a later connection. So does the kernel's `HID_NAME`.
