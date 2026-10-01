# Parity with Apple: what is sent to the keyboard, and nothing more

Decision of the manager: implement the functions Apple's system calls for this keyboard (A1314, PID `0x0256`), **only** what Apple really sends, with the same bytes and in the same context. Sources: `RE-PILOTE-MACOS.md`, `RE-PILOTES-ANCIENS.md`, `RE-MACOS-SILICON.md`, `RE-GHIDRA-IOBLUETOOTH.md`, `RE-GHIDRA-KEXT.md` (branch `re/ghidra-kext`).

## What is implemented

| Apple does | Bytes / context | Here | Issue |
|---|---|---|---|
| `handleShutdown` / `handleRestart` -> `willShutdown()` -> `setExtendedReport("WillShutdown", NULL, 0)` | SET Feature `0x40`, the id alone: `53 40` on the wire; at each shutdown or restart | `parity::will_shutdown`, once per run, if `[apple] will_shutdown` (default **true**) and the keyboard is connected; circuit breaker, 1 s spacing; logind `PrepareForShutdown` inhibitor + `akmctl shutdown-notify` (D-Bus `NotifyShutdown`) + user unit | #191 |
| `IOAppleBluetoothHIDDriver::processInterruptData`: `A1 30 xx` -> `updateBatteryState` | Input `0x30`, 0 normal, 1 low, 2-3 critical; passive listening | explicit "keyboard alert" labels, signal, CapsLock flash, deduplicated with the percentage alerts | #189 |
| `AppleBluetoothHIDKeyboard::processInterruptData`: `A1 13 xx`, bit 1 = 0 -> `KeyboardOff`, then the `Disconnected` notification is dropped | Input `0x13`; passive listening | `PassiveEvent::KeyboardOff`, `PoweredOff`, `LinkEvent::PoweredOff` | #190 |
| `bluetoothd` `FUN_1006a1ae0` (`prepareForSleep`) / `FUN_1006a2108` (`prepareForWake`) | HID_CONTROL SUSPEND `13` before sleep, EXIT_SUSPEND `14` at wake, on the HIDP control channel; once per device, wait ≤ 1 s | `akm-hid-control` (root) duplicates the control socket (PSM `0x0011`) of `bluetoothd` with `pidfd_getfd` and sends the one byte; system units `apple-kb-monitor-suspend.service` (`Before=sleep.target`) / `apple-kb-monitor-resume.service`; `enabled` in `/etc/apple-kb-monitor/hid-suspend.conf` (default **true**); test: `akmctl hid-control … --dry-run` ([VEILLE-HID.md](VEILLE-HID.md)) | #244 |

## What is deliberately not implemented

* `0x4A` (SCO link state, #216), `0x41` `RecantConnection`, `0x44` `FullFactoryDefault` (#217), `0x45`, `0x50`-`0x55`, `0xD0`-`0xFB`, `0x09` (#171), `0xD5`: forbidden or out of scope. The register map refuses them.

## Why only `0x40` (Feature) and `0x13` / `0x14` (HID_CONTROL) can be written

* `registry::check_write` accepts one pair: Feature `0x40` (class `WriteAppleParity`); the test `only_will_shutdown_is_writable` sweeps the 256 ids in the three directions.
* `registry::WriteSession` allows one write per run and refuses data after the id; the test sweeps the 256 ids and checks that a second write is refused.
* `hidraw::hid_write_feature` is the only function that issues `HIDIOCSFEATURE`, sized for one byte, checked against `check_write`, every byte logged on stderr (`[hid-write] ...`) before the ioctl; a source scan (`this_build_has_one_write_path`) fails if another file names a write ioctl.
* HID_CONTROL (#244) is a separate program and a separate path: `akm-hid-control` can emit only `0x13` or `0x14` (`HidControl` enum + `check_byte`; the test `only_0x13_and_0x14_pass_the_256_sweep` sweeps the 256 values), one `send` of one byte per keyboard, on the single connected L2CAP socket of `bluetoothd` whose peer is the keyboard and whose PSM is `0x0011`; `this_file_has_one_write_path` fails on any other write call. Apple context: RE-GHIDRA-KEXT §2.4 notes that macOS emits `0x14` directly only for HID devices handled in user space; under BlueZ the keyboard is one (`uhid`), so both bytes are sent.
* All tests use a spy (`FeatureSink`) or an invalid descriptor (`-1`): no test ever writes to a real keyboard.

## Effect and risk

The effect of `WillShutdown` on the keyboard is not observable without writing (RE-PILOTE-MACOS §8): Apple's expected gain is a clean sleep without reconnection attempts during the shutdown. Risk: low (production traffic of macOS). To opt out: `[apple] will_shutdown = false`.
