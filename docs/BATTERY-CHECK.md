# Adversarial check of the battery percentage — A1314 ISO (BCM2042)

Date: 2026-10-01. Device: Apple Wireless Keyboard A1314 ISO, `AA:BB:CC:DD:EE:F1`, `0005:05AC:0256`,
firmware `0x0050`, 2 AA batteries inserted around 03:00 (keyboard offline 02:55 → 03:06).
The author's question: is the 98-99 % shown a few hours after the change accurate?
Approach: try to **refute** that figure.

Evidence levels: **[measured]** (hardware or log of this machine), **[source]** (cited reference),
**[model]** (computation on a published, approximate curve), **[hypothesis]**.

Strictly read-only: `HIDIOCGFEATURE` (GET_REPORT) only, node opened `O_RDONLY`, no SET_REPORT,
no write, no sudo, no service touched. The pairing report is neither read nor quoted.

## 0. Verdict

| Question | Answer | Margin |
|---|---|---|
| Was the morning's 98-99 % too high? | **No, refutation failed.** By charge balance, batteries inserted 9 h earlier consumed 9 to 45 mAh out of 2,000 to 2,850 mAh, i.e. **98.2 to 99.6 % actual** [model]. The figure was right in substance. | actual 97-100 % |
| And the 96 % at 12:29? | **Too low.** `0x47` only drops in steps, on reconnections (§1.2bis). It lost 3 points between 11:00 and 12:29 during link disturbances, without matching consumption. `0xEA` stays at 98, and the table applied to `0x49` gives 98.9. | −3 points due to the link |
| Does the figure come from a fresh measurement? | **Yes.** `power_supply/capacity` is not a cache: each read causes a radio GET_REPORT of `0x47` (kernel code + btmon). UPower polls it every 30 s. No 5 s kernel period. | — |
| What does it really measure? | A **voltage** (`0x49` = 2,935-2,953 mV, i.e. ~1.47 V per cell), apparently taken at reconnection, projected onto the factory table `0x5A`. Above 2,954 mV the firmware shows 100 %, so the top of the scale says almost nothing. | 1 point = 17.9 mV per pair |
| Will it stay accurate? | **No: it becomes optimistic.** The table puts 75 % at 1.253 V per cell, which corresponds to **~35 % actual** for an alkaline; 50 % → ~25 %; 25 % → ~5 % [model]. | ± 10 points |
| Is the chemistry consistent? | 2.97-2.99 V a few hours after insertion: **alkaline** (or zinc-carbon). Charged NiMH would read 2.70-2.80 V; new lithium would read 3.3-3.5 V. | — |
| Battery life? | The first day's slope is **not** usable (relaxation of new batteries, radio kept active by the reads). The previous set still showed **90 % firmware after ~180 days**, i.e. roughly 62-65 % actual. | see §4 |

**What to show the user**: the firmware percentage, labelled "keyboard indication"; a separate estimate
according to the chemistry, with its range; a battery life in days, computed only from the long history
of a battery set; a warning if the voltage at insertion does not match the declared chemistry.
See §6.

## 1. Measurement chain

### 1.1 Where `0x47` comes from

```
battery ×2 ─► BCM2042 ADC (step ≈ 4.4-4.5 mV) ─► 0x46 / 0xFF  instantaneous voltage (noise ±1 step)
                                              └► 0x49        "slow" voltage (noise-free, ~36 mV below 0x46)
                       factory table 0x5A = 0x60 = 0xEB: 2954 / 2506 / 2404 / 2054 mV  (100/75/50/25 %)
                                              └► 0xEA, 0x47  integer percentages (0x47 = Battery Strength,
                                                              read by the kernel as Feature, quirk PERCENT|FEATURE)
```

* **[measured]** `0x46` (u16 LE) = `0xFF[1..3]` (u16 BE), in mV, within one ADC step. The observed values
  (2974, 2978, 2982, 2986, 2991) are 4 to 5 mV apart: this is the ADC quantization.
* **[measured]** `0x49` never shows noise: 2953 (03:57), 2950 (04:15-05:15, 13 reads), 2945 (11:37-12:02,
  11 btmon reads and the 12:02 sweep). It stays **36-38 mV below the mean of `0x46`** both 1 h and
  8 h apart: so it is not a simple low-pass filter of `0x46`, which would have converged.
  **[hypothesis]** Voltage under load (sampled during a radio transmission) or a held minimum value.
* **[measured]** Table `0x5A` differs from the code default (2900/2450/2350/2000) by +54/+56/+54/+54 mV.
  This almost constant offset looks like a **unit-specific ADC calibration**, applied to the thresholds
  rather than to the measurement [hypothesis]. Consequence: the absolute voltage in volts has an uncertainty
  of the order of ± 50 mV per pair, but the comparison to the thresholds is correct.

### 1.2 Reconstruction of the voltage → % function

Function tested: `pct = trunc(linear_interp(0x49 ; 0x5A ↔ 100/75/50/25))`, capped at 100
(`tests/live/re/verif_battery_model.py`, `fw_pct`).

| Time | `0x49` mV | Computed % (exact) | `0x47` | `0xEA` | kernel / UPower | Agreement |
|---|---|---|---|---|---|---|
| 03:07-03:26 | not read | — | 100 (history) | — | 100 | — |
| 03:57 | 2953 | 99.94 | 99 | 98 | 99 | ✅ 0x47 |
| 04:15-05:15 | 2950 | 99.78 | 99 | 98 (a 0 at 04:40) | 99 | ✅ 0x47 |
| 11:37-12:02 | 2945 | **99.50** | **98** (137 btmon reads) | 98 | 98 | ❌ expected 99 |
| 12:30:49 (after reconnection 12:29) | 2935 | **98.94** | **96** | 98 | 96 (UPower) | ❌ expected 98 |

* **[measured] Partial refutation**: the hypothesis of `HARDWARE-HID-REPORTS.md` §4 ("`0x47` = truncation
  of the interpolation of `0x49`") gives 99 at 2945 mV, while the keyboard has returned 98 for at least 25 min.
  Neither truncation nor rounding (99.497 → 99) explains it.
* **[hypothesis]** `0xEA` would be the raw estimate and `0x47` its displayed value with delay or hysteresis:
  `0xEA` is 98 from 03:57 and `0x47` joins it between 10:52 and 11:33 (daemon history). Another lead:
  the firmware computes on an internal quantity it does not expose. No read-only access can settle it.
* **Practical scope: ± 1 point.** The shape of the function (cap at 100 above 2954 mV, slope of
  17.9 mV per point down to 75 %) is solid. The criterion "`floor(pct_fine)` = `0x47` ± 1" of the fine percentage remains
  satisfied.

### 1.2bis `0x47` only drops on reconnections [measured: correlation; mechanism: hypothesis]

Each step of `0x47` follows a disconnection or reconnection of the keyboard (BlueZ and daemon logs):

| Link event | `0x47` before → after | `0x49` at that time | Table % |
|---|---|---|---|
| disconnection 03:25:47, reconnection before 03:41 | 100 → **99** (03:41:47) | ~2953 | 99.9 |
| disconnection 04:00:33, reconnection before 04:15 | 99 → 99 | 2950 | 99.8 |
| bluetoothd restarts 11:15-11:16, `Host is down` 11:32:29 | 99 → **98** (11:33:02) | 2945 | 99.5 |
| disconnection 12:13:28, reconnection 12:29:31 | 98 → **96** (12:29:31) | 2935 | 98.9 |

During a continuous session, `0x47` stays fixed: 99 from 04:15 to 10:52 while `0x49` goes from 2950 to
~2946, then 98 during 137 reads from 11:37 to 12:02. **[hypothesis]** The firmware computes `0x47` at
(re)connection time, on a voltage taken under the load of radio *paging*, and does not recompute it
during the session. 96 % corresponds to ~2882-2900 mV on the table, i.e. 35 to 50 mV below `0x49`: this is
the order of magnitude of the ohmic drop of a transmission peak.

Consequences:
* **the kernel percentage does not drop with consumption, but in steps on reconnections.**
  The 3-point loss between 11:00 and 12:29 comes from the day's link disturbances (BlueZ restarts,
  massive reads by other study tools, the 12:13 drop-off). It does not come from consumption:
  45 mAh in 1.5 h would require 30 mA continuously;
* on this day, `0x47` is therefore rather **pessimistic** (96); `0xEA` stays at 98 and the table applied
  to `0x49` gives 98.9;
* the margin of `0x47` is at least −3 points because of link events, on top of the
  non-linearity of the scale (§2).

### 1.3 Freshness: cache or measurement?

* **[source]** `hid-input.c` (`hidinput_get_battery_property`): if the state is not `HID_BATTERY_REPORTED`
  and `avoid_query` is false, **each read of `capacity` issues a GET_REPORT**; otherwise the kernel returns
  the last value received through an input report. The only time limit in the code is the 30 s limit
  of `hidinput_update_battery`, which only applies to spontaneous input reports.
* **[measured]** btmon capture from 11:37 to 12:02 (`re-link/stats1.json`, neighbouring capture): 137 answers
  `DATA type=3 (Feature) id=0x47` and **no `0x47` input report** on the interrupt channel. The state
  therefore stays "queried", never "received": `capacity` is **always a fresh radio measurement**.
* **[measured]** Intervals between `0x47` requests: groups of 2 or 3 requests spaced **30 s** apart
  (43 intervals ≈ 30 s, 81 intervals of 0 to 1 s): this is UPower polling, not the kernel.
* The "5 s cadence" is the daemon's `TICK` (`actor.rs:119`), which republishes a snapshot and does not
  re-read the keyboard every 5 s. The daemon does a full read every 15 min
  ("acquired …" in the log: 11:33, 11:48, 12:03).
* **[hypothesis] Measurement artefact**: this morning, the radio was kept awake by about 32 GET_REPORTs
  per minute (812 in 25 min, several study tools on top of UPower). The day's consumption and the
  day's voltage slope are therefore **not representative** of normal use.

## 2. Physical model

Voltage per cell = measured voltage / 2. Low-rate discharge curves (~1 mA, 21 °C), **approximate**
(± 0.03 V, ± 10 points): synthesis of the Energizer E91 (alkaline), L91 (Li-FeS₂), Eneloop/NH15 (NiMH) datasheets and
[Wikipedia AA battery](https://en.wikipedia.org/wiki/AA_battery). The E91 and L91 datasheets only give their curves
as graphs: the table values are read by eye, not copied. Internal resistance: alkaline
150-300 mΩ, L91 120-240 mΩ [source, datasheets]. At a few mA, the ohmic drop is less than 3 mV per pair,
so the voltage read ≈ open-circuit voltage.

Consumption of a BT 2.0 keyboard [qualitative source, BCM2042 datasheet: "battery life > 6 months"]:
about 0.1 mA in deep sleep, 0.5 to 1 mA connected at rest, a few mA while typing. Average order
of magnitude: 0.3 to 1 mA in normal use.

Table produced by `verif_battery_model.py` (actual state = 100 − depth of discharge, at the given voltage):

| Pair (mV) | V/cell | Firmware % | Alkaline actual | NiMH Eneloop actual | Lithium L91 actual |
|---|---|---|---|---|---|
| 2991 | 1.496 | 100 | ~89 | (impossible at rest) | ~13 |
| 2950 | 1.475 | 99 | ~84 | (impossible at rest) | ~11 |
| 2935 | 1.468 | 98 | ~82 | — | ~10 |
| 2900 | 1.450 | 96 | ~78 | — | ~9 |
| 2775 | 1.388 | 90 | ~62 | ~99 | ~7 |
| 2600 | 1.300 | 80 | ~44 | ~92 | ~4 |
| **2506** | 1.253 | **75** | **~35** | ~63 | ~4 |
| **2404** | 1.202 | **50** | **~25** | ~21 | ~3 |
| 2200 | 1.100 | 35 | ~10 | ~4 | ~2 |
| **2054** | 1.027 | **25** | **~5** | ~1 | ~1 |

Reading by chemistry, for 2.98-2.99 V (≈ 1.49 V per cell):

* **Alkaline**: on the voltage curve, ~85-90 % actual. But **the charge balance prevails for new
  batteries**: 9 h × 1 to 5 mA = 9 to 45 mAh out of ~2,500 mAh, i.e. more than 98 % actual. The gap comes from the
  surface plateau (1.58-1.62 V when new), which disappears in the very first percent. **98-99 % is plausible.**
  A truly new alkaline rather reads around 1.52-1.55 V per cell under light load: 1.49 V suggests
  either batteries stored for a long time, or zinc-carbon cells, or the ADC offset of §1.1.
* **NiMH (Eneloop)**: 1.49 V per cell is impossible at rest (1.35-1.42 V right after charging, plateau
  at 1.20-1.25 V). With NiMH, the firmware would show **85-90 % from insertion**, then **50-75 % for
  almost the whole life** of the batteries, and 25 % at the very end: the percentage would be wrong the other way.
* **Lithium (L91)**: 1.49 V per cell corresponds to **end of life** (less than 15 % left). New, they
  would read 3.3 to 3.5 V per pair: within the 1.7-3.6 V VBAT range of the BM2042 module datasheet (the "2.7-3.3 V" mention
  of the Broadcom *brief* is a schematic label, probably the regulator output); small margin, not verified on the board. They would show 100 % for
  ~90 % of their life, then collapse within a few days.

→ **Only alkaline (or zinc-carbon) makes 98-99 % consistent** with 2.97-2.99 V a few hours after insertion.

## 3. Time series (03:07 → 12:15, 9 h, without additional keyboard reads)

The planned "3 h live" series was replaced by the measurements already recorded, which are longer:

* at 12:13:12, the first burst of `verif_battery_series.py` received an EIO (`hidp_report_req_timeout`)
  and the tool stopped by itself. The keyboard disconnected at 12:13:28, during the reads
  of another tool;
* instruction at 12:20: suspension. Resume only if `Connected: yes`, with
  4 reports (`0x46`, `0x49`, `0x47`, `0xEA`) once every 5 min and stop at the first error. The bursts
  obtained after that time are in `tests/live/re/verif_series_20261001.jsonl` (§3.3).

### 3.1 Percentage (daemon history, `~/.config/apple-kb-monitor/history.jsonl`, and btmon)

| Window | `0x47` / kernel | Duration |
|---|---|---|
| 03:07:45 → 03:26 | 100 % | ~20-35 min |
| 03:41:47 → 10:52 | 99 % | ~7 h 10 |
| ≤ 11:33:02 → 12:13 (disconnection) | 98 % | — |

The April set behaved the same way: **100 % at 00:43 on 04/04, then 98 % from 10:52**,
and 98 % during the following 2.5 days. The 100 → 98 descent in less than 10 h **repeats** from one battery
set to the next: it is the voltage relaxation of new batteries, not a 2 % consumption
(which would require ~50 mAh in 8 h, i.e. 6 mA continuously).

### 3.2 Voltages

| Window | n | `0x46` mean (σ) | `0xFF` mean | `0x49` | `0x46` − `0x49` |
|---|---|---|---|---|---|
| 03:57 (audit) | 2 | 2991 | 2982-2991 | 2953 | 38 |
| 04:15-05:15 (fixture) | 13 | 2988.3 (2.5) | 2987.5 | 2950 | 38 |
| 11:37-11:51 (btmon) | 11 | 2981.6 (2.7) | 2982.0 | 2945 | 37 |
| 12:02 (neighbouring sweep) | — | 2974-2982 | 2969-2978 | 2945 | ~33 |

* Quantization: ADC step ≈ 4.4-4.5 mV; `0x46` noise = ±1 step (σ ≈ 2.5 mV). `0x49` moves in
  steps of 3 to 5 mV, about every few hours.
* `0x47` moves in whole-point steps: 1 point = 17.9 mV per pair on the 100-75 % segment.
  At the day's slope, that is **one step every 20 to 25 h**.
* **Slope** (centres 04:45 → 11:44, 7 h): `0x46` −0.93 mV/h, `0x49` −0.71 mV/h.

### 3.3 Light bursts after resuming

`verif_series_20261001.jsonl`: 11 bursts from 12:30:49 to 13:21:13, one every 5 min, 4 reports per
burst, without any error. The keyboard had reconnected by itself at 12:29:31. The sampler was
stopped by hand at 13:21 and no process remains active.

| Time | `0x46` mV | `0x49` mV | `0x47` | `0xEA` | UPower |
|---|---|---|---|---|---|
| 12:30:49 | **2978** | 2935 | 96 | 98 | 96 % |
| 12:35:52 | 2982 | 2935 | 96 | 98 | 96 % |
| 12:40:55 | 2982 | 2935 | 96 | 98 | 96 % |
| 12:45:57 → 13:21:13 (8 bursts) | 2986 | 2935 | 96 | 98 | 96 % |

* `0x46` **rises** from 2978 to 2986 mV in the 15 min after reconnection: this is the recovery
  of an alkaline after a current draw. It supports the hypothesis of a voltage taken under load
  at reconnection time (§1.2bis).
* `0x49` stays fixed at 2935 and `0x47` at 96 for 50 min: no rise of `0x47` within the session,
  which confirms it is frozen until the next reconnection. No measurable slope in 50 min.
* UPower = `0x47` at every burst: no stale value.

### 3.4 Battery life estimate from the slope: **not valid**

Extending −0.71 mV/h down to 2054 mV (25 % firmware, ~5 % actual) gives ~1,250 h, i.e. **~52 days**.
This is **wrong by construction**:
1. an alkaline loses voltage much faster early in its life than on its plateau;
2. today the radio was kept awake by more than 30 requests per minute;
3. the previous set lasted at least 180 days (§4).

A slope only makes sense after several weeks, and only on `0x49` (noise-free, fine step).

## 4. Previous battery set

Files: `~/.local/share/apple-kb-monitor/history.jsonl.bak` (04/04 00:06 → 11:37, 1,264 lines) and
`history.jsonl` (04/04 11:38 → 01/10 03:22, 872 lines); `~/.config/apple-kb-monitor/history.jsonl`
(= `~/.local/state/…`, symbolic link; 01/10 02:33 → 12:03, 46 lines). The `voltage` field is the constant
`0xF5` (2.9777 / 2.9806 / 2.9032 V): **it is not used**.

| Date | % | Note |
|---|---|---|
| 04/04 00:06 | 98 | old set |
| 04/04 00:43 | 100 | **set inserted** [hypothesis: jump 98 → 100] |
| 04/04 10:52 → 07/04 | 98 | same relaxation as today |
| 07/04 → 01/10 | **no data** | daemon stopped or history not written: 177-day gap |
| 01/10 02:33 → 02:54 | **90** | last state of the old set |
| 01/10 03:07 | 100 | current set inserted |

* **Impossible to calibrate a time-% relation on a full discharge**: there is none in
  the history, only two points (98 % on 07/04, 90 % on 01/10) separated by 177 days without measurement.
* What can be drawn from it [model]: 90 % firmware ⇒ `0x49` between 2775 and 2793 mV ⇒ 1.388-1.397 V per
  cell ⇒ **about 62-65 % actual (± 10)** for an alkaline. **The batteries removed on 01/10 probably still had
  more than half of their capacity.**
* If the set was indeed inserted on 04/04 and used regularly, ~35-40 % consumed in 180 days gives a
  life of **15 to 17 months** (wide range, 10 to 24 months). This value is uncertain: actual usage
  during the gap is not known.
* The BCM2042 datasheet announces "more than 6 months", and the old set at 90 % after 6 months agrees:
  **the firmware percentage was not contradicted by the previous set**, it is simply very slow to
  drop at the top of the scale.

## 5. Readable reports: anything missed?

The 27 IDs of the map (`HARDWARE-HID-REPORTS.md` §2), checked against all of the day's measurements (sweep,
04:15-05:15 series, btmon 11:37-12:15, neighbouring 12:02 sweep):

| Varies with time or load | Used? |
|---|---|
| `0x46` instantaneous voltage | yes in `akm-core` (02fad76); the Python CLI still reads `0xF5` |
| `0xFF` bytes 1-2 = `0x46` | duplicate of `0x46`; byte 3 = `0x01` constant (flag to watch at end of life) |
| `0x49` slow voltage | planned with the fine percentage; **this is the quantity the firmware follows** |
| `0x47` % | yes (kernel, daemon) |
| `0xEA` % (ahead of `0x47`, transient 0) | no (overridden by the kernel on the Rust side); useful as an early sign of the next step |
| `0xFE` (`fe0004` once, then zeros) | no: **not** a tool artefact (counter-audit, see HARDWARE-HID-REPORTS §2), meaning unknown; **stop reading it** (the keyboard did not answer it just before the 12:13 drop-off) |

Constant over 9 h: `0x09`, `0x4A` (18), `0x4B`, `0x4F`, `0x51-0x54`, `0x5A`/`0x60`/`0xEB`, `0x5B`, `0x5C`,
`0x5D`, `0xD1`, `0xD8`, `0xF4`, `0xF5`, `0xF6`, `0xF7`. **No variable report was missed.**
`0x4A` = 18 could have been a temperature in °C [weak hypothesis], but it stayed at 18 from 03:57 to
12:02: to be checked once on a hot day, without additional reads.

Unused derived quantity: **the `0x46` − `0x49` gap** (36-38 mV, stable). If `0x49` is indeed a voltage
under load, this gap follows the internal resistance, which increases as an alkaline wears out: it is a
possible health indicator.

## 6. What to display

1. **Firmware percentage** (`0x47`, = kernel): keep it, with the label "keyboard indication".
   Never present it as a linear remaining charge.
2. **Separate estimate**: `0x49` projected onto the curve of the declared chemistry (alkaline by default),
   with its range (± 10 points). During the first 2 days of a set: "new batteries", without a figure
   derived from the voltage.
3. **Battery life in days**: only from a history of at least 2 weeks on `0x49`, or from the
   measured life of the previous set; never from the first day's slope.
4. **Chemistry warning**: `0x49` < 2.85 V right after insertion → NiMH likely, the firmware % will be
   low all its life; `0x46` > 3.15 V → lithium, the % will stay at 100 until the end, then collapse.
5. **Alerts**: the daemon's 30/15/5 % thresholds apply to the firmware scale. **30 % firmware ≈ 7 %
   actual** for an alkaline: the first alert comes too late (dedicated issue).

## 7. Follow-up

| Type | Subject |
|---|---|
| bug | 30/15/5 % alerts on the firmware scale: the 1st alert comes at ~7 % actual |
| doc/RE | `0x47` ≠ trunc(interp(`0x49`)) at 2945 mV; `0x49` ≠ filter of `0x46`: fix the map and the fine percentage issue |
| bug | history: `voltage` = constant `0xF5` over 2,136 lines, to be marked unreliable and replaced by `0x46`/`0x49` |
| comment | do not regress on the firmware % nor on the first days' slope |
| comment | `BatteriesInstalledAt` = 0 after restart; 300 mV threshold too high (actual jump of +160 to +180 mV) |
| comment | chemistry rules at insertion, firmware → actual table, `0x46` − `0x49` gap |
| comment | exact equality refuted; the fine % stays on the firmware scale |

## 8. Reproduce

```bash
python3 tests/live/re/verif_battery_model.py                      # voltage → % table per chemistry, no hardware
python3 tests/live/re/verif_battery_model.py --mv 2945 2775       # specific points
python3 tests/live/re/verif_battery_series.py --analyze tests/live/re/verif_series_20261001.jsonl
# light mode, waits for Connected: yes, 4 reports / 5 min, stop at the 1st error:
python3 tests/live/re/verif_battery_series.py AA:BB:CC:DD:EE:F1 --rounds 36 --period 300 --wait-node --out s.jsonl
```

Sources: [hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c)
(`hidinput_get_battery_property`, `hidinput_query_battery_capacity`, quirk `ALU_WIRELESS_2011_ISO`);
[Energizer E91](https://data.energizer.com/pdfs/e91.pdf); [Energizer L91](https://data.energizer.com/pdfs/l91.pdf);
[Wikipedia — AA battery](https://en.wikipedia.org/wiki/AA_battery); BCM2042 datasheet (see maintainer notes).
