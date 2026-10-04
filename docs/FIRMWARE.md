# Firmware version check

The daemon reads Feature report `0x4F` (little-endian u16; `4f 50 00` = `0x0050`, equal to the `bcdDevice` of the
modalias) **only once per connection**, then compares the version with an **embedded table** of the latest
known public version per product identifier (PID). No network request, no flashing: the result is
information only.

## What the program does

| Step | Detail |
|---|---|
| Read | `0x4F` (registry class `OncePerConnection`, `akm-core/src/registry.rs`) after the three routine reads, with the 1 s spacing, the circuit breaker at 3 failures and the single lock of `read_policy`. The "already requested" flag is reset on every `note_connection()`. A failure does not retry the read within the same connection: the status stays `unknown`. |
| Table | `akm-core/src/firmware.rs`, `KNOWN_FIRMWARE`, review date `TABLE_DATE`. |
| Status | `up_to_date`: version = latest known; `update_available`: version ∈ {`0x0044`, `0x0046`} (old versions for which Apple's 2009 updater is documented); `unknown`: model missing from the table, version not read, version newer than the table, or another older version (the table cannot conclude and says so). |
| No flashing | No write function exists (`registry::check_write` refuses everything). "Update available" means "Apple published a newer version for this model". |

## Where it is exposed

* D-Bus (root and each keyboard's object): `FirmwareVersion` (`0x0050`, empty = not read yet), `FirmwareLatestKnown`
  (empty = model outside the table), `FirmwareStatus` (`up_to_date` / `update_available` / `unknown`, never empty).
* JSON (`akmctl status --json`, `Json` property): `firmware.version_hex`, `firmware.latest_known`, `firmware.status`,
  `firmware.source`, `firmware.table_date`.
* `akmctl status` (`Firmware:` line), `akmctl firmware` (version, latest known, status, source, table date),
  `akmctl firmware --json`.
* System Settings module, State page: version and status (translated). Plasma widget: a line in the full view and in the tooltip.

## Where each table entry comes from

| PID | Latest version | Source |
|---|---|---|
| `0x0255`, `0x0256`, `0x0257` (A1314 ISO/ANSI/JIS) | `0x0050` | **[measured]** `0x4F` = `4f 50 00` on an A1314 ISO `05AC:0256` on 2026-10-01 (docs/HARDWARE-HID-REPORTS.md §2). **No known public update program for these PIDs** (maintainer notes): "latest known" = the version observed on the hardware, not a version Apple would distribute. |
| `0x0239`, `0x023A`, `0x023B` (A1255, 2007-2009) | `0x0050` | **[public source]** "2009 Aluminum Keyboard Firmware Update" (support.apple.com/en-us/106755): `Parameters.plist` `FWVersion = 80` (= `0x50`), target 0x239-0x23B (maintainer notes). Earlier versions: `0x44` / `0x46`. |

The other models (A1255 `0x022C`-`0x022E`, Magic Keyboard…) are `unknown`: the table does not know them, and the daemon
does not read any vendor report on families other than BCM2042 anyway.

## Updating the table

1. Get the evidence: a version read on a real keyboard (`akmctl info` after connecting) or the Apple support page /
   an updater's `Parameters.plist`. A version seen only once on one unit is not "the latest public one".
2. Add or change the entry in `KNOWN_FIRMWARE` (`akm-core/src/firmware.rs`) with a precise `source`; add a
   documented older version to `OLDER_WITH_UPDATE` only if Apple's updater is established for that PID.
3. Update `TABLE_DATE` (ISO date) and the table above.
4. `cargo test -p akm-core firmware::`: `pids_are_unique_across_the_table`, `current_firmware_of_every_known_pid_is_up_to_date`.
   Add the case to the tests if a new status rule appears.

## Report registry (summary)

`akmctl info` lists the full table. Safety classes: `SafeRead` (`0x47`, `0x46`, `0x49`), `OncePerConnection`
(`0x4F`, `0x60`, `0x51`-`0x54`; the daemon reads `0x4F` and `0x60`, then `0x51`-`0x54` at low priority for the own name), `PassiveInput` (`0x04` `0x05` `0x30` `0x13` `0x11`
`0x12`), `ManualOnly` (readable but never requested by the daemon: `0xFF`, `0x5A`, `0xEB`, `0x5B`, `0xF4`, `0xF5`, `0xEA`,
`0x09`, `0x5C`, `0x5D`), `NeverRead` (`0xFE`, `0x4C`, Input `0x01`, `0x34`, `0x35`), `WriteApple` (written only by a named Apple operation: `Shutdown` = `0x40`, `Forget` = `0x41` (`akmctl repair` only), `DeviceName` = `0x55`, real write refused until proven), `NeverWrite` (`0x43`
`0x44` `0x45` `0x4A` `0x50`, `0xD0` `0xD4` `0xD5` `0xFA` `0xFB`, and any unknown `0xDx`/`0xFx`), `Unknown`. The read
whitelist is **generated** from the table; `hid_read_feature` (the only `ioctl` call towards the keyboard) refuses any id whose class
does not allow reading, before the ioctl. Only one HID write is effective: `0x40` `WillShutdown` (id alone, once at shutdown, what macOS sends,
`docs/APPLE-PARITY.md`); `0x55` (own name, `docs/RENAME-KEYBOARD.md`) is written only by `akmctl rename --device-name` (guarded flow) and does not pass the one-byte door; `hid_write_feature` is the only `HIDIOCSFEATURE` call. `0x41` `RecantConnection` is only sent by `akmctl repair` (clean forget of a connected keyboard); `0x44` (forget all hosts) and
`0x4A` (SCO notification) remain refused and are not implemented.

`0x60` thresholds (Full / Low / Critical / Empty, mV, 4 × u16 BE, read once per connection): exposed in JSON
(`battery.thresholds`, `threshold_level`, `threshold_margins_mv`) and shown with the remaining margin before `Low` and `Critical`
. They do not replace the chemistry-based estimate: these are the firmware thresholds, the estimate remains that of the
declared chemistry.
