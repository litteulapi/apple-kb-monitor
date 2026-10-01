# Parity with Apple: what is sent to the keyboard, and nothing more

Decision of the manager: implement the functions Apple's system calls for this keyboard (A1314, PID `0x0256`), **only** what Apple really sends, with the same bytes and in the same context. Sources: `RE-PILOTE-MACOS.md`, `RE-PILOTES-ANCIENS.md`, `RE-MACOS-SILICON.md`, `RE-GHIDRA-IOBLUETOOTH.md`, `RE-GHIDRA-KEXT.md` (branch `re/ghidra-kext`).

## What is implemented

| Apple does | Bytes / context | Here | Issue |
|---|---|---|---|
| `handleShutdown` / `handleRestart` -> `willShutdown()` -> `setExtendedReport("WillShutdown", NULL, 0)` | SET Feature `0x40`, the id alone: `53 40` on the wire; at each shutdown or restart | `parity::will_shutdown`, once per run, if `[apple] will_shutdown` (default **true**) and the keyboard is connected; circuit breaker, 1 s spacing; logind `PrepareForShutdown` inhibitor + `akmctl shutdown-notify` (D-Bus `NotifyShutdown`) + user unit | #191 |
| `IOAppleBluetoothHIDDriver::processInterruptData`: `A1 30 xx` -> `updateBatteryState` | Input `0x30`, 0 normal, 1 low, 2-3 critical; passive listening | explicit "keyboard alert" labels, signal, CapsLock flash, deduplicated with the percentage alerts | #189 |
| `AppleBluetoothHIDKeyboard::processInterruptData`: `A1 13 xx`, bit 1 = 0 -> `KeyboardOff`, then the `Disconnected` notification is dropped | Input `0x13`; passive listening | `PassiveEvent::KeyboardOff`, `PoweredOff`, `LinkEvent::PoweredOff` | #190 |
| `bluetoothd` `FUN_1006a1ae0` (`prepareForSleep`) / `FUN_1006a2108` (`prepareForWake`) | HID_CONTROL SUSPEND `13` before sleep, EXIT_SUSPEND `14` at wake, on the HIDP control channel; once per device, wait ≤ 1 s | `akm-hid-control` (root) duplicates the control socket (PSM `0x0011`) of `bluetoothd` with `pidfd_getfd` and sends the one byte; system units `apple-kb-monitor-suspend.service` (`Before=sleep.target`) / `apple-kb-monitor-resume.service`; `enabled` in `/etc/apple-kb-monitor/hid-suspend.conf` (default **true**); test: `akmctl hid-control … --dry-run` ([VEILLE-HID.md](VEILLE-HID.md)) | #244 |
| `bluetoothd` `FUN_1005a3f64` -> `FUN_1005a41e4` ("Oublier" a connected classic Apple HID): SET `RecantConnection`, wait 2000 ms for the drop, then unpair | SET Feature `0x41`, id alone, `53 41` on the wire [décompilé + listing]; only when the device is connected; effect on the keyboard **not measured** | `akmctl repair` only (operation `Forget`, once per session), after pre-flight, host-side backup (no link key) and the typed `OUBLIER`; the daemon mutes the disconnection (`ExpectDisconnect`); 2000 ms; then `RemoveDevice`; if `0x41` fails nothing is removed (`docs/RECONNEXION-PAIRAGE.md` §5.4) | #217 |

## What is deliberately not implemented

* **Name stored in the keyboard (`0x55` `LongDeviceName`, #248).** Prepared (validation, backup, frames, dry run: `akmctl rename --device-name`), registered as the named operation `DeviceName`, but the real write is **refused** (`NotProven`): Lion 10.7 sends one SET Feature `0x55` of 64 bytes [désassemblage], yet the bytes of the name field are not established and macOS 26.5 never writes the name. See `RENOMMER-CLAVIER.md` §5.
* `0x4A` (SCO link state, #216), `0x44` `FullFactoryDefault` (Lion's forget; macOS 26.5 sends `0x41` instead, #217), `0x45`, `0x50`-`0x54`, `0xD0`-`0xFB`, `0x09` (#171), `0xD5`: forbidden or out of scope. The register map refuses them.

## What can be written, and how

* Class `WriteApple` + named operations (`registry::WriteOp`): `Shutdown` = Feature `0x40` (id only), `DeviceName` = Feature `0x55` (exactly 64 data bytes; real write refused until proven), `Forget` = Feature `0x41` (id only, `akmctl repair` only). `registry::check_write` accepts only these ids, `check_write_op` only the ids of the given operation; the test `only_the_ids_of_the_named_operations_are_writable` sweeps the 256 ids in the three directions and compares with the list of the operations.
* `registry::WriteSession` allows each id once per session, refuses another operation's id and any length but the operation's exact one; `session_authorises_each_id_once_with_its_exact_length` sweeps the 256 ids for every operation.
* `hidraw::hid_write_feature(fd, op, report)` is the only function that issues `HIDIOCSFEATURE`, sized for **one** byte: an operation that carries data (`DeviceName`) can never pass it. Every byte is logged on stderr (`[hid-write] ...`) before the ioctl; a source scan (`this_build_has_one_write_path`) fails if another file names a write ioctl. `hidraw::WriteDoor` is the same door opened by a short-lived `akmctl` command under the HID lock shared with the daemon.
* HID_CONTROL (#244) is a separate program and a separate path: `akm-hid-control` can emit only `0x13` or `0x14` (`HidControl` enum + `check_byte`; the test `only_0x13_and_0x14_pass_the_256_sweep` sweeps the 256 values), one `send` of one byte per keyboard, on the single connected L2CAP socket of `bluetoothd` whose peer is the keyboard and whose PSM is `0x0011`; `this_file_has_one_write_path` fails on any other write call. Apple context: RE-GHIDRA-KEXT §2.4 notes that macOS emits `0x14` directly only for HID devices handled in user space; under BlueZ the keyboard is one (`uhid`), so both bytes are sent.
* All tests use a spy (`FeatureSink`) or an invalid descriptor (`-1`): no test ever writes to a real keyboard.

## Effect and risk

The effect of `WillShutdown` on the keyboard is not observable without writing (RE-PILOTE-MACOS §8): Apple's expected gain is a clean sleep without reconnection attempts during the shutdown. Risk: low (production traffic of macOS). To opt out: `[apple] will_shutdown = false`.
