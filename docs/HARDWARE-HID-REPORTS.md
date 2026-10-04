# HID Feature reports of the Apple A1314 keyboard (BCM2042) — reverse-engineering map

* Corrected on 2026-10-02 : Apple names of the reports carried over to §2, §2.1 (new) and §6: `0x09`, `0x13`, `0x30`, `0x40`, `0x41`, `0x43`, `0x44`, `0x45`, `0x50`, `0x55` (source: maintainer notes).
* Corrected on 2026-10-02 : §4, the % = f(mV) function is internal to the firmware; macOS copies `0x47` without conversion (source: maintainer notes).
* Corrected on 2026-10-02 : §2, §4 and §6, `0x5A`/`0x60`/`0xEB` are not "100/75/50/25 % thresholds" but Full / Low / Critical / Empty; `0x49` = `BatteryVoltage`, `Latched` voltage (source: maintainer notes).

Device studied: Apple Wireless Keyboard A1314 ISO "Clavier de alice #1", `AA:BB:CC:DD:EE:F1`,
`0005:05AC:0256`, bcdDevice `0x0050`, kernel driver `apple`, `/dev/hidraw7`.
New batteries fitted on 2026-10-01 around 03:00 (keyboard offline from 02:55 to 03:06, kernel capacity 90 % → 100 %).

Method: **read only** (`HIDIOCGFEATURE` = GET_REPORT, node opened `O_RDONLY`),
no SET_REPORT, no sudo, at most one burst of reads per 5 minutes.
Tools: `tests/live/re/sample_reports_scan.py`, `tests/live/re/idle_timeout.py`.
Data: `tests/fixtures/a1314_iso/reports_scan_20261001.json` (sweep 0x00-0xFF),
`tests/fixtures/a1314_iso/reports_timeseries_20261001.json` (time series).

Levels of evidence:

* **[measured]**: observed on the hardware during this study;
* **[source]**: cited public document or code;
* **[hypothesis]**: plausible interpretation, not proven.

> ⚠️ Report `0x4C` contains a secret (19 bytes, high entropy). It is never copied in clear:
> the files only contain its length, its first byte and a truncated SHA-256 fingerprint.

## 1. What the HID descriptor declares

224-byte descriptor (`/sys/class/hidraw/hidraw7/device/report_descriptor`, identical to
`tests/fixtures/a1314_iso/report_descriptor.bin`) decoded **[measured]**:

| Report ID | Type | Declared content |
|---|---|---|
| `0x01` | Input/Output | boot keyboard: 8 modifiers, 1 reserved byte, 6 keys; 5 output LEDs |
| `0x47` | **Input** | page `0x06` *Generic Device Controls* / usage `0x20` Battery Strength, 8 bits, 0-255 (in a Consumer collection `0x0C:0x01` → Generic Desktop Keyboard) |
| `0x11` | Input | 3 padding bits, Consumer Eject `0xB8`, vendor `0x00FF:0x03` (= Fn key), 3 padding bits |
| `0x12` | Input | Consumer Play/Pause `0xCD`, Fast Forward `0xB3`, Rewind `0xB4`, Scan Next `0xB5`, Scan Previous `0xB6` + 3 padding bits (no Stop `0xB7`) |
| `0x13` | Input | vendor `0xFF01:0x0A` (1 bit, `Input 0x02`), `0xFF01:0x0C` (1 bit, `Input 0x22` = Data,Var,**Abs**,No Preferred — not relative), 6 padding bits |
| `0x09` | **Feature** | vendor `0xFF01:0x0B`, 1 data byte + 2 constant bytes (padding) |

**Only `0x09` is declared as Feature.** `0x47` is declared as Input, but the kernel reads it as Feature:
`hid-input.c` lists the wireless aluminum Apple keyboards in `hid_battery_quirks` with
`HID_BATTERY_QUIRK_PERCENT | HID_BATTERY_QUIRK_FEATURE` ("ask for feature report") **[source]**
([hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c)).
All the other IDs answer GET_REPORT without being declared: they are internal registers
of the Broadcom firmware, exposed through the HIDP control channel **[measured]**.

`hid-apple.c` sends no Feature report to the 0x0256: its reads/writes (`0xBF`, `0xB0`, battery
timer `APPLE_RDESC_BATTERY`) target backlighting and the USB Magic Keyboards; for
`USB_DEVICE_ID_APPLE_ALU_WIRELESS_2011_ISO` it only sets `APPLE_NUMLOCK_EMULATION | APPLE_HAS_FN |
APPLE_ISO_TILDE_QUIRK` **[source]** ([hid-apple.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-apple.c)).
No public project found documents the BCM2042 vendor IDs (WebSearch searches 2026-10-01);
the rest of this document therefore rests on measurement.

## 2. Full sweep 0x00-0xFF

27 IDs answer (the others return an error). Length = bytes returned by the ioctl, ID included.
IDs `0x54`, `0x5C`, `0x5D`, `0xD1`, `0xD8` were not in the initial inventory.

| ID | Len. | Raw (2026-10-01) | Decoding | Evidence |
|---|---|---|---|---|
| `0x09` | 4 | `09 01 00 00` | byte 1 = `0x01` (vendor `FF01:0B`) = **firmware Caps Lock delay flag**, `01` = delay disabled; bytes 2-3 = padding | descriptor [measured]; meaning [disassembly] maintainer notes (macOS writes `01` on the 2007 keyboards only, never on the 598, and applies its 75 ms delay on the host side); meaning of `00` [deduction] |
| `0x46` | 3 | `46 aa 0b` / `46 af 0b` | **u16 LE = battery voltage in mV**: 2986 / 2991 mV | [measured], see §3 |
| `0x47` | 2 | `47 63` | u8 = 99 % (Battery Strength) | [source] descriptor + kernel quirk; = `power_supply/capacity` [measured] |
| `0x49` | 3 | `49 89 0b` → `49 86 0b` | **`BatteryVoltage`** (Apple name): u16 LE = 2953 then 2950 mV, varies slowly | live quantity [measured]; `Latched` voltage for Apple [plist + disassembly 10.7.5] maintainer notes, see §4 |
| `0x4A` | 2 | `4a 12` | u8 = 18 | meaning on read unknown; on write, state of the SCO link set by the host (1 to 4) [disassembly] maintainer notes |
| `0x4B` | 3 | `4b 00 08` | `00 08` | unknown |
| `0x4C` | 20 | `4c 03` + 18 masked bytes | 1 byte `0x03` + **BD_ADDR of the paired host** (6 B, LE) + 12 secret bytes | address [measured, §2bis]; 12 remaining B SENSITIVE [hypothesis] link key fragment |
| `0x4F` | 3 | `4f 50 00` | u16 LE = `0x0050` = firmware version/bcdDevice | = `Modalias usb:v05ACp0256d0050` and the kernel's "HID v0.50" [measured] |
| `0x51` | 9 | `51` + `"Clavier "` | name, fragment 1/4 (8 B ASCII); `0x51-0x54` = `DeviceName1..4` at Apple | [measured]; Apple name [plist] |
| `0x52` | 9 | `52` + `"de alice"` | name, fragment 2/4 | [measured] |
| `0x53` | 9 | `53` + `" #1"` + NUL | name, fragment 3/4 | [measured] |
| `0x54` | 9 | `54` + 8 × NUL | name, fragment 4/4 (empty here) | [measured]; name ≤ 32 bytes |
| `0x5A` | 9 | `5a 0b8a 09ca 0964 0806` | 4 × u16 BE: 2954, 2506, 2404, 2054 mV = **Full / Low / Critical / Empty** thresholds (Apple names, given for `0x60`) | values [measured]; names [plist + disassembly 10.7.5] maintainer notes |
| `0x5B` | 9 | `5b 06cc 0384 00000000` | concatenation of `0xF4` and `0xF5` + 4 zeros | [measured] |
| `0x5C` | 9 | 8 × `00` | empty | [measured] |
| `0x5D` | 9 | 8 × `00` | empty | [measured] |
| `0x60` | 9 | = `0x5A` | **`CalibratedBatteryThresholds3`** (Apple name): Full / Low / Critical / Empty; same content as `0x5A` | [measured]; name [plist] maintainer notes |
| `0xD1` | 2 | `d1 00` | u8 = 0 | unknown |
| `0xD8` | 2 | `d8 00` | u8 = 0 | unknown |
| `0xEA` | 2 | `ea 62` (once `ea 00`) | u8 = 98, transient 0 | [hypothesis] second % estimator; isolated 0 to be ignored [measured] |
| `0xEB` | 9 | = `0x5A` | copy of `0x5A` | [measured] |
| `0xF4` | 3 | `f4 06 cc` | u16 BE = 1740 | constant; [hypothesis] see §4 |
| `0xF5` | 3 | `f5 03 84` | u16 BE = 900 | **constant across a battery change: it is not the voltage** [measured] |
| `0xF6` | 3 | `f6 00 04` | 4 | unknown |
| `0xF7` | 3 | `f7 00 04` | 4 | unknown |
| `0xFE` | 9 | 8 × `00` | empty except once | [measured] `fe 00 04` in the `--dump` of 03:57:18, then zeros in the 16 exact committed reads (sweep, rounds A/B, 13 bursts). **Not a tool artifact**: the `--dump` of the time (before 2219efa) allocated a fresh buffer per ID and only listed a report if a payload byte was non-zero; the `04` therefore comes from the answer [counter-audit]. Meaning unknown; **do not read any more** |
| `0xFF` | 4 | `ff 0b aa 01` / `ff 0b af 01` | **u16 BE = same voltage as `0x46`** + byte `0x01` | [measured], see §3 |

Endianness: `0x46`, `0x49`, `0x4F` are little-endian; `0x5A`/`0x5B`/`0xF4`/`0xF5`/`0xFF` big-endian.
The firmware therefore mixes two conventions; `0x46` (LE) and `0xFF` (BE) give the same quantity.

### 2.1 Reports named by Apple outside the 27 readable IDs (added on 2026-10-02)

Apple's IOKit personality for PID 598 (`ExtendedFeatures`) and the disassembly of the macOS 26.5 driver name
reports that this Feature sweep could not see: Input, or write-only Feature (maintainer notes).

| ID | Type | Apple name | Meaning | Our keyboard | Evidence |
|---|---|---|---|---|---|
| `0x13` | Input (declared, §1) | — | bit 1 = "powered": an `A1 13 xx` report whose bit 1 is 0 triggers `KeyboardOff` | — | [disassembly] |
| `0x30` | **Undeclared** Input | `BatteryState` | 0 normal, 1 low, 2-3 critical; read by GET and also **pushed as an interrupt** (`A1 30 xx`) | `30 00` | [plist + disassembly]; value [measured] maintainer notes |
| `0x40` | Feature, write only | `WillShutdown` | the host is about to shut down | GET refused `0x03` | [plist] |
| `0x41` | Feature, write only | `RecantConnection` | give up the connection | GET refused `0x03` | [plist] |
| `0x43` | Feature | `UserMode` (1-3) | declared by Apple, **absent** from our firmware | `ERR_INVALID_REPORT_ID` (`0x02`) | [plist] + [measured] |
| `0x44` | Feature, write only | `FullFactoryDefault` | full reset | GET refused `0x03` | [plist] |
| `0x45` | Feature, write only | `FactoryDefault` | reset | GET refused `0x03` | [plist] |
| `0x50` | Feature, write only | `DeviceNameChange` | commit of a name change | GET refused `0x03` | [plist] |
| `0x55` | Feature, write only | `LongDeviceName` | long name, 64 bytes | GET refused `0x03` | [plist] |

The `0x03` refusals (ERR_UNSUPPORTED_REQUEST) are measured in the maintainer notes
Still unknown to macOS: Feature `0xD0 0xD4 0xFA 0xFB` and Input `0x04`/`0x05`. `0xD5` is only cited by a CoreBluetooth radio
test tool (maintainer notes). `0x4E`, `0xC6` and `0xDC`, named by IOBluetooth for other
products, are absent from this keyboard (maintainer notes).

### 2bis. Structure of `0x4C` (without disclosure)

* **[measured]** bytes 2 to 7 = `aa:bb:cc:dd:ee:f2` read backwards, i.e. the address of the PC's Bluetooth
  adapter (`HID_PHYS`), not the keyboard's. Checked by boolean comparison, without displaying
  the rest.
* **[measured]** 12 remaining bytes: 12 distinct values out of 12 (high entropy), SHA-256 fingerprint
  identical on every read of the day (stable register).
* **[hypothesis]** pairing record: byte `0x03` (type or slot number),
  bonded host, then key material (12 bytes do not make a full 16-byte link key).
* Correction: it is **not** an IRK (BLE concept; the A1314 is BR/EDR). The
  "identity key" label of the code is inaccurate, but the masking rule still fully applies.
* The Python CLI (`read_all_reports`) publishes `d[2:16]` (14 bytes): the host address **and 8 of the
  12 secret bytes**; the Rust daemon publishes the whole report.

## 3. Actual battery voltage: `0x46` and `0xFF`, not `0xF5`

* **[measured]** In the same burst of reads, `0x46` read as LE and `0xFF[1..3]` read as BE give the same
  value (sweep of 03:57:29: 2986/2986; round A of 03:58:24-33: 2991/2991). The initial inventory dump
  (03:57:18, non-simultaneous reads) showed 2991 (`0x46`) and 2982 (`0xFF` = `0b a6`): the quantity moves between two
  reads, which a configuration constant does not do.
* **[measured]** 2.99 V for two new alkaline AA batteries (≈ 1.50 V/cell): physically consistent.
* **[measured]** `0xF5` is `0x0384` (900) **before and after the battery change** of 2026-10-01
  (history `~/.config/apple-kb-monitor/history.jsonl`: "voltage" 2.9032 V = 900 × 3.3 / 1023 at 90 %
  at 02:33 with the old batteries, then at 100 % at 03:07 with the new ones). A battery voltage
  cannot stay identical to the millivolt between a worn set (90 %) and a new set.
  In April 2026 the history gave 924 (2.98 V) for 2 days without the slightest variation.
* Consequence: the "voltage" computed as `adc_raw * 3.3 / 1023` (`0xF5`) is wrong. The "build number"
  `0x0BAA` is actually 2986 mV. **State at the counter-audit (c54c502)**: `akm-core` is fixed (02fad76, voltage = `0x46`);
  the Python CLI keeps the old decodings.

## 4. Thresholds, curves and percentages

* `0x5A` = `0x60` = `0xEB` (three identical copies **[measured]**): 2954 / 2506 / 2404 / 2054 mV,
  strictly decreasing, consistent with the curve of a pair of alkalines (1.48 / 1.25 / 1.20 / 1.03 V
  per cell). The project's contract (`calibration.rs`) maps them to 100/75/50/25 % **[project source,
  not verified]**. **Corrected**: these are not 100/75/50/25 % thresholds. Apple names the four values of `0x60`
  (`CalibratedBatteryThresholds3`) **Full / Low / Critical / Empty** [plist + disassembly 10.7.5]; `Low` and `Critical`
  would match states 1 and 2 of `BatteryState` [deduction] (maintainer notes). Values specific to the unit (the code default is 2900/2450/2350/2000).
* Consistency check **[measured]**: with 2986-2991 mV (> 2954), the 100/75/50/25 curve would give 100 %,
  yet `0x47` = 99 and `0xEA` = 98. On the other hand `0x49` = 2953 mV, interpolated on the same curve, gives 99.94 %
  → 99 when truncated = `0x47`. **[hypothesis]** `0x49` is the filtered voltage (or measured under radio load)
  from which the firmware derives the percentage, `0x46`/`0xFF` the instantaneous measurement. **Corrected**: Apple names `0x49`
  `BatteryVoltage` and publishes it as a `Latched` voltage, measured then frozen by the firmware [plist + disassembly
  10.7.5] (maintainer notes). That the percentage is derived from it remains a hypothesis.
* `0xEA` (98) ≠ `0x47` (99): **[hypothesis]** second estimator (another filter or another rounding);
  the old "percentage before rounding" interpretation is incompatible with 98 < 99.
* **Corrected**: the % = f(mV) function is **entirely internal to the firmware**. macOS converts nothing: its kernel
  driver reads `0x47`, clamps it to 100 and publishes it as is **[disassembly]** (maintainer notes), 60 s after connection
  then every 4 h (1 h after a failure). The discrepancy noted in the battery check (the interpolation of `0x49` gives 99.5, the keyboard returns 98;
  maintainer notes) is therefore also what a Mac would publish: the `BatteryPercent` property and Settings, which
  read the raw %. Only the legacy IOBluetoothUI battery icon goes through a host-side display curve (54 to 100
  raw → 100 %), which does not start from a voltage (maintainer notes).
* `0xF4` = 1740, `0xF5` = 900 (duplicated in `0x5B`), constant **[measured]**. Competing hypotheses:
  (a) 1740 mV = cut-off voltage (0.87 V/cell, below the last threshold 2054);
  (b) 900 = idle delay in seconds (15 min) before sleep — tested in §5;
  (c) radio parameters (sniff) in 0.625 ms slots (1087.5 ms / 562.5 ms).

## 5. Time observation (read only)

Main series: 13 bursts of 27 GET_REPORTs, one every 5 min, 2026-10-01 04:15:26 → 05:15:38,
no gap and no I/O error (`reports_timeseries_20261001.json`). Supplemented by the capture of the
neighboring audit (`tests/live/re/capture-2026-10-01.jsonl`, branch `audit/decodage-hid`, 03:57-04:00).

| Time | `0x46` mV | `0xFF` mV | `0x49` mV | `0x47` % | `0xEA` | kernel % |
|---|---|---|---|---|---|---|
| 03:57:18 (neighbor) | 2991 | 2982 | 2953 | 99 | 98 | 99 |
| 03:58:24 (round A) | 2991 | 2991 | 2953 | 99 | 98 | 99 |
| 04:15:26 | 2986 | 2986 | 2950 | 99 | 98 | 99 |
| 04:25:27 | 2991 | 2986 | 2950 | 99 | 98 | 99 |
| 04:40:31 | 2986 | 2986 | 2950 | 99 | **0** | 99 |
| 05:05:36 | 2991 | 2991 | 2950 | 99 | 98 | 99 |
| 05:15:38 | 2991 | 2991 | 2950 | 99 | 98 | 99 |

Findings **[measured]**:

1. **Constant throughout the day** (sweep, series, neighboring capture): `0x09`, `0x4A`, `0x4B`, `0x4C`
   (identical fingerprint), `0x4F`, `0x51-0x54`, `0x5A`/`0x60`/`0xEB`, `0x5B`, `0x5C`/`0x5D`, `0xD1`/`0xD8`,
   `0xF4`, `0xF5`, `0xF6`/`0xF7`, `0xFE`.
2. **`0x46` and `0xFF`** oscillate between 2982, 2986 and 2991 mV: steps of 4 to 5 mV (probable quantization step;
   the ADC resolution is published nowhere [hypothesis]), noise of ± 1 step. The two registers are read ~10 s apart within a burst, hence one-step differences.
3. **`0x49`** goes from 2953 (03:57) to 2950 (from 04:15) then stays fixed for an hour: smoothed quantity,
   without the noise of `0x46` → supports "filtered voltage".
4. **`0xEA`** is 98 except for **one read at 0** (04:40:31, `ea00`, correct length) while `0x47` and the
   kernel stay at 99: transient value (recomputation in progress?). Any consumer must ignore an isolated 0.
   The April 2026 history also shows occasional 0 % values (an older bug, different cause: failed read).
5. `0x4C` always designates the host `aa:bb:cc:dd:ee:f2`.
6. Battery: 99 % kernel/`0x47` over the whole hour; one hour of new batteries is not enough to see
   the percentage move, only the voltage is informative.

Sleep and disconnection:

* The neighboring capture shows the link lost in the middle of a burst at 04:00:13 (`errno 5` after ~3.5 s per
  report), BlueZ logs "keyboard disconnected" at 04:00:33 then `Host is down`: the keyboard went
  to sleep. No SET_REPORT was sent; the cause (idle sleep or effect of the reads) is not
  settled.
* From 04:15 to 05:15, with one read every 5 min, the link stayed up.
* **[hypothesis]** `0xF5` = 900 s = idle delay before sleep. Proposed test (passive, without requests):
  `tests/live/re/idle_timeout.py` timestamps the input reports (content never kept) and the
  disappearance of the node; a last keypress → disconnection gap ≈ 900 s over several cycles, without any
  GET_REPORT in the meantime, would confirm it. Not run to completion here (reads suspended on
  instruction during the disconnection).

> **Counter-audit**: `0xEA` "second estimator" and the relation `0x47` = trunc(interp(`0x49`))
> are contradicted at 2945 mV (BATTERY-CHECK §1.2). Apple names of several IDs (`0x30` BatteryState,
> `0x40` WillShutdown, `0x41` RecantConnection, `0x44`/`0x45` FactoryDefault, `0x50` DeviceNameChange, `0x51-0x54`
> DeviceName1..4, `0x55` LongDeviceName): maintainer notes. `0x09`: probable flag of the internal Caps Lock delay
> (maintainer notes, deduction).

## 6. Summary: decoded / decodable / unknown

| Status | Reports |
|---|---|
| **Decoded** (measured evidence) | `0x46` instantaneous voltage mV (LE) · `0xFF` same voltage (BE) + byte `0x01` · `0x47` % (= kernel) · `0x4F` version `0x0050` · `0x51-0x53` ASCII name · `0x4C` bytes 2-7 = paired host · `0x5A`/`0x60`/`0xEB` table of 4 voltages (3 copies) · `0x5B` = `0xF4`‖`0xF5` · `0x5C`/`0x5D`/`0xFE` empty · `0xF5` is **not** a voltage · `0x09` = firmware Caps Lock delay flag, `01` = disabled (meaning established by the Apple driver, §2) |
| **Decodable** (hypothesis testable without writing) | `0x49` = `BatteryVoltage`, Apple's `Latched` voltage (live, noise-free) · `0x5A`/`0x60`/`0xEB` = Full / Low / Critical / Empty thresholds (Apple names; not 100/75/50/25 %) · `0xEA` second % estimator, transient 0 · `0xF5` = 900 s sleep delay (protocol §5) · `0xF4` = 1740 mV cut-off · `0x54` 4th name fragment (name ≤ 32 B; the code's truncation at 24 B not demonstrated, current name 19 B) · byte 3 of `0xFF` (flag, to watch at end of battery life) · `0x4C` byte `0x03` |
| **Unknown** (constant, no correlation possible by reading) | `0x4A` on read (18; Apple only writes the SCO state 1 to 4 there) · `0x4B` (`00 08`) · `0xD1`/`0xD8` (0) · `0xF6`/`0xF7` (4) · last 12 bytes of `0x4C` (secrets, not studied) |

The constant reports can only be elucidated by writing (SET_REPORT), which is excluded, or by
comparing several keyboards / firmwares (another A1314, A1255), or by observing the end of battery life
(`0xFF` byte 3, `0xEA`, `0x49` below 2054 mV).

## 7. User features made possible and tracking

| Feature | Reports | Tracking |
|---|---|---|
| Fix the displayed voltage (`0xF5` × 3.3/1023 → `0x46` mV) and the wrong decodings (`0xFF` "build", `0x46`/`0x49` "BT parameters", `0xEA` "pre-rounding") | `0x46`, `0xFF`, `0x49`, `0xEA` | tracked |
| Real voltage + fine 0.1 % percentage on the unit's factory curve | `0x46`, `0x49`, `0x5A` | tracked |
| Paired host read from the keyboard, re-pairing alert elsewhere | `0x4C` bytes 2-7 only | tracked |
| Battery replacement detection by voltage jump | `0x49`/`0x46` | tracked |
| Battery health / type (alkaline vs NiMH), instantaneous − smoothed gap | `0x46`, `0x49`, `0x5A` | tracked |
| Earlier runtime forecast (the voltage moves before the whole %) | `0x49` | tracked |
| Integrity check of the calibration table (3 copies) | `0x5A`, `0x60`, `0xEB` | already in the code (`calib_mirror_*`) |
| Sleep delay displayed / predicted | `0xF5` | not opened: hypothesis to confirm (§5) |
| Charge state, power mode | none: the A1314 runs on batteries, no report varies with a power supply | not applicable |

## 8. Reproduce

```bash
python3 tests/live/re/sample_reports_scan.py --scan-once                    # sweep 0x00-0xFF but 0x4C and 0xFE, once
python3 tests/live/re/sample_reports_scan.py --interval 300 --count 13 --out s.jsonl
python3 tests/live/re/sample_reports_scan.py --analyze s.jsonl              # summary, without hardware
python3 tests/live/re/idle_timeout.py --duration 7200 --out idle.jsonl # passive, timestamps only
```

Sources: [hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c) (Apple battery quirks),
[hid-apple.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-apple.c) (0x0256 quirks),
[BCM2042 (product sheet)](https://www.alldatasheet.com/html-pdf/175090/BOARDCOM/BCM2042/384/1/BCM2042.html) (BT HID chip with battery interface, no public register table).
