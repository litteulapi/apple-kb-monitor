# Keyboard inputs and model coverage

Audit of 2026-10-01, test PC (Manjaro, kernel 7.1.13, keyd v2.6.0, BlueZ 5.87), real keyboard
A1314 ISO "Clavier de alice #1" `AA:BB:CC:DD:EE:F1` (`0005:05AC:0256`, driver `apple`).
Code base: `main` @ `a2b7359`. Deliverable branch: `re/entrees-modeles`.

**Method: read only.** No write to the keyboard (no SET_REPORT, no output report,
no EV_LED), no sudo, no service stopped. Hardware access limited to:
`/sys` (uevent, `report_descriptor`, `capabilities`, LED `brightness`, `hid_apple` parameters)
and `evtest /dev/input/event28` in capabilities mode (the node was already grabbed by keyd: evtest
reported "grabbed by another process" and received no event). The keyboard went to sleep
at 04:00:33; no hardware access after that. No key event sampling was
done: what is marked "not observed" was not seen on the wire.

Sources: `drivers/hid/hid-apple.c`, `hid-ids.h`, `hid-input.c`, `hid-magicmouse.c`
(torvalds/linux master of 2026-10-01), kernel selftest `tools/testing/selftests/hid/tests/test_apple_keyboard.py`,
`rvaiya/keyd` master (`daemon.c`, `device.c`, `vkbd/uinput.c`), gist xloc/9f1ecca9 (descriptor of the
2021 Magic Keyboard). Fixtures and provenance: `tests/fixtures/models/README.md`.

Tests without hardware: `python3 -m pytest tests/live/re -q` (21 tests, all passing). Descriptor parser: `tests/live/re/hid_rdesc.py <file>`.

---

## 1. Input side of the A1314 (BCM2042) — measured

Captured descriptor (224 B) **identical byte for byte** to the descriptor of the kernel selftest
`AppleKeyboard` (`test_identical_to_kernel_selftest`).

| Report | Type | Content (bit: usage) | Kernel (hid-apple / hid-input) | Driver today |
|---|---|---|---|---|
| `0x01` | input 8 B | modifiers E0–E7, 1 reserved B, 6 codes (0–255) | keys; Fn translation according to `fnmode` | not read (the passive listener reads it in the stream and discards it) |
| `0x01` | output 1 B | bits 0–4: NumLock, CapsLock, ScrollLock, Compose, Kana (page `0x08`) | `input51::{numlock,capslock,scrolllock,compose,kana}` | Rust: sysfs read + EV_LED write (§3); Python: raw hidraw write |
| `0x11` | input 1 B | bit 3 `000C:00B8` Eject; bit 4 `00FF:0003` Fn | `KEY_EJECTCD`, `KEY_FN` | ignored |
| `0x12` | input 1 B | bits 0–4: play/pause, fast forward, rewind, next track, previous | `KEY_PLAYPAUSE`, `KEY_FASTFORWARD`, `KEY_REWIND`, `KEY_NEXTSONG`, `KEY_PREVIOUSSONG` | ignored; actual emission **not observed** (F7–F9 go through report 0x01 + Fn table) |
| `0x13` | input 1 B | bit 0 `FF01:000A`, bit 1 `FF01:000C` (NoPref) | not mapped (vendor) | Rust: counts every `0x13` without decoding; Python: decodes the 2 bits |
| `0x47` | input 1 B | `0006:0020` Battery Strength 0–255 | `power_supply hid-<mac>-battery-71` (71 = 0x47), 99 % read | battery (kernel first) |
| `0x09` | feature 3 B | `FF01:000B` + 2 constant B | — | read ("device state") |

evdev capabilities (`tests/fixtures/a1314_iso/evtest_capabilities.txt`, 182 codes): EV_SYN, EV_KEY,
EV_MSC (MSC_SCAN), EV_LED (5 LEDs). Special keys present: `KEY_FN`, `KEY_EJECTCD`,
`KEY_BRIGHTNESSDOWN/UP`, `KEY_SCALE`, `KEY_DASHBOARD`, `KEY_KBDILLUM{DOWN,UP,TOGGLE}`,
`KEY_PREVIOUSSONG`, `KEY_PLAYPAUSE`, `KEY_NEXTSONG`, `KEY_FASTFORWARD`, `KEY_REWIND`, `KEY_MUTE`,
`KEY_VOLUMEDOWN/UP`, `KEY_SEARCH`, `KEY_MICMUTE`, `KEY_SLEEP`, `KEY_POWER`, `KEY_STOPCD`
(hid-apple declares the union of all its tables; the A1314 emits only part of it, see below).

**Fn state** (`/sys/module/hid_apple/parameters`, readable without privileges): `fnmode=1`, `iso_layout=-1`,
`swap_opt_cmd=0`, `swap_ctrl_cmd=0`, `swap_fn_leftctrl=0`. `fnmode=1` comes from `modprobe/hid_apple.conf`
(redundant with the default 3 = auto → 1 for an Apple keyboard, already noted in `modprobe/hid_apple.conf`). No driver
code reads these parameters (writing is done by `akm-helper set-fnmode`; reading was proposed for D-Bus).

Fn table applied by the kernel to the 9 BCM2042 PIDs (`magic_keyboard_alu_fn_keys`, fnmode 1 = media
by default, F-key with Fn): F1 brightness −, F2 +, F3 `KEY_SCALE`, F4 `KEY_DASHBOARD`,
**F5 without translation**, **F6 `KEY_NUMLOCK`**, F7–F9 media, F10–F12 volume, Fn+⌫ = Delete,
Fn+↵ = Insert, Fn+arrows = PgUp/PgDn/Home/End. The 9 PIDs have `APPLE_NUMLOCK_EMULATION`: when the NumLock
LED is on at kernel level, J/K/L/U/I/O/M… become keypad keys (hid-apple l.575).

## 2. keyd and remapping — read in the code and in /etc/keyd

`/etc/keyd/apple-keyboard.conf` = the repository's `keyd/apple-keyboard.conf` (identical).
`[ids] 05ac:0256` only. keyd (active pid) grabs `event28` (EVIOCGRAB) and re-emits on
"keyd virtual keyboard" (`input49`).

Effective mapping with `fnmode=1` on the A1314:

| Key | Without Fn → keyd | With Fn → keyd |
|---|---|---|
| F3 | `KEY_SCALE` → `M-z` | `KEY_F3` → `M-z` |
| F4 | `KEY_DASHBOARD` → `M-g` | `KEY_F4` → **not remapped** |
| F5 | `KEY_F5` → `M-l` | `KEY_F5` → `M-l` |
| F6 | `KEY_NUMLOCK` → `M-d` | `KEY_F6` → `M-d` |
| `kbdillumdown/up` | never emitted by the A1314 (dead lines) | — |
| Eject | `KEY_EJECTCD` → not remapped | — |

Defects: keyd (ids limited to one PID, asymmetric F4, 2015/2021 tables not covered: on a
2021 Magic Keyboard F6 = `KEY_SLEEP`).

## 3. LED via keyd — code check (without writing any LED)

Rust path (`akm-core/src/led.rs`): `led_target_in()` → keyd virtual keyboard if it exists, otherwise
the Apple evdev found by `HID_ID`. Write of a 24 B EV_LED `input_event`. keyd
(`daemon.c`) relays every EV_LED received by its virtual device to **all the devices
it has grabbed** via `device_set_led()` (write on their grabbed fd): the path is correct
**for the A1314 0x0256**. State read: LED `inputN::capslock` whose HID parent is an Apple
model (correct, ignores `input49` and `input36`). Only caller: `flash_capslock(5)` (battery
alert); nothing writes NumLock.

State read: `input49::capslock=0`, `input51::capslock=0`, `numlock` 0/0 → consistent at the time
of the audit.

Defects:
1. keyd target chosen even if keyd does not grab the keyboard (every PID ≠ 0x0256 with the
   shipped configuration) → flash silently lost;
2. the input core ignores an EV_LED equal to the current state of the virtual keyboard: when out of sync, the
   first toggle is swallowed;
3. `set_led(0, true)` (public API) would trigger hid-apple's NumLock emulation on BCM2042.

Python `--led`: raw output report on hidraw and state aggregated over all LEDs of the system.

## 4. Models × features matrix

Legend: **OK** supported and proven · **P** partial · **NO** missing/broken · **NT** not tested
(deduced from the code/sources, no hardware) · "—" not applicable.

| Family (PID) | Rust detection | Python detection | Kernel battery | Vendor HID telemetry | RSSI | Fn / fnmode | Eject | Wake 0x13 | CapsLock LED (flash) | keyd remap |
|---|---|---|---|---|---|---|---|---|---|---|
| **A1314 2011 ISO `05AC:0256`** (real) | OK (test `real_a1314_iso_uevent`) | OK | OK (99 %, `-71`) | OK (other audit) | OK (helper) | kernel OK; driver NO | kernel OK; driver NO | P (counted, not decoded) | OK via keyd (code) | OK (F4 partial) |
| A1314 2011 ANSI/JIS `0255/0257` | OK (table) | OK (name) | NT (same descriptor assumed) | NT | NT | same | same | NT | **NO** if keyd active | **NO** |
| A1314 2009 `0239–023B` | OK | **NO** (missing) | NT | NT | NT | NT | NT | NT | NO if keyd | NO |
| A1255 `022C–022E` | OK | **wrong** (022C "JIS") / missing | NT | NT | NT | NT | NT | NT | NO if keyd | NO |
| A1016 white (2003) | NO: PID missing from hid-ids.h, `hid-generic` driver, no hid-apple Fn | "0x0220 = A1016" **wrong** (it is the wired ALU) | NT | — | NT | — | — | NT | NO if keyd | NO |
| Magic Keyboard 2015 A1644/A1843 `0267/026C` BT `004C` | OK (MagicKeyboard family, no vendor feature) | **NO** (vendor 004C ignored; 0267 labeled "Touch ID") | NT (BT: battery only if the descriptor declares it; USB: `APPLE_RDESC_BATTERY` + kernel GET_REPORT every 60 s) | — (on purpose) | NT | kernel `magic_keyboard_2015_fn_keys` | no key | not applicable (unknown descriptor) | NO if keyd | NO |
| Magic Keyboard 2021 A2450 `029C` | OK | NO | NT; published descriptor: report `0x90` Battery page (`0085:0065` charge, `0085:0044` charging) → power_supply + charge status | — | NT | Fn = `FF01:0003` in `0x01` → `KEY_FN` | no key; Lock key `000C:019E` → `KEY_COFFEE` (= SCREENLOCK), not used | **no 0x13 report**: monitor useless | NO if keyd | NO (F6 = `KEY_SLEEP`) |
| Touch ID 2021 A2449/A2520 `029A/029F` | OK | NO | NT | — | NT | NT | — | NT | NO if keyd | NO |
| Magic Keyboard 2024 USB-C `0320–0322` | OK | NO | NT | — | NT | NT | — | NT | NO if keyd | NO |
| USB cable (any MK, `0003:05AC`) | OK (table) but 1st hidraw of the PID, several interfaces possible; `HID_UNIQ` without `:` → no MAC → no kernel battery found by MAC; BlueZ reports nothing → machine never triggered | NO (bus 0003 excluded) | kernel OK (hid-apple timer) | — | — | — | — | — | NT | NO |
| Magic Mouse/Trackpad 1 `05AC:030D/030E` | rejected OK; **but** the watcher takes them for a keyboard | **taken for a keyboard** + vendor feature probes | — | — | — | — | — | — | — | — |
| Magic Mouse/Trackpad 2 (+USB-C) `004C:0269/0265/0323/0324` | rejected OK; watcher fixed; MM2 mouse report = `0x12` (≠ A1314 media) | ignored (vendor) | hid-magicmouse (USB); BT: not read | — | — | — | — | — | — | — |
| AirPods / iPhone (Modalias `004C`) | watcher: **Connected → keyboard hidden** | — | — | — | — | — | — | — | — | — |

Evidence for the "Rust detection" column: `test_17_pid_egaux_a_hid_ids`,
`test_family_matches_kernel_name`, `test_detection_per_case` (25 cases with `HID_ID` from
`families.json`), plus the existing unit tests of `model.rs`. The Rust table (17 PIDs,
vendors 05AC and 004C) **matches hid-ids.h**; both vendors are accepted for
each PID (the kernel itself registers the BCM2042 only as `05AC` BT and the Magic Keyboards only as
`004C` BT / `05AC` USB: no consequence, the reverse combination does not exist on the wire).

udev rule `70-apple-kb-hidraw.rules`: covers all the positive cases
(`test_udev_uaccess_couvre_les_claviers`) but also mice, trackpads and any Apple USB HID
(`test_udev_trop_large_documente`): `uaccess` on their hidraw. No open issue (impact
limited to the seat user); to be tightened if the model table becomes the reference.

## 5. Missing user features

| Feature | Hardware basis | Tracking |
|---|---|---|
| Read and display the effective `fnmode` | sysfs `hid_apple/parameters/fnmode` | tracked |
| Live Fn state (pressed) | `0x11` bit 4 / `KEY_FN` | tracked |
| Usable Eject key (e.g. Delete, lock) | `0x11` bit 3 / `KEY_EJECTCD` | tracked |
| Decoded wake (ready / connection request) | `0x13` bits 0/1 | tracked |
| Lock key of MK 2021+ | `000C:019E` | tracked |
| CapsLock indicator | sysfs LED, already read | tracked |
| Per-family remap (2015/2021 tables) | hid-apple tables | tracked |
| USB wired mode (interface, serial number instead of MAC) | `0003:05AC` | tracked |
| Trackpad/Mouse battery | `0x90` / hid-magicmouse | tracked |
| Hardware validation of untested families | descriptors to capture (§6) | tracked |

## 6. To capture when a model is available (read only)

`/sys/class/hidraw/hidrawN/device/report_descriptor`, `uevent`, `bluetoothctl info`,
`/sys/class/power_supply/hid-*`, `evtest` in capabilities mode. Put them under
`tests/fixtures/models/<family>/` and fill in `rdesc` in `families.json`: the tests
`test_rapport_0x13_seulement_bcm2042` and following will apply automatically.

## 7. Defects found

| Type | Subject |
|---|---|
| bug | watcher: any Apple device (AirPods, mouse) hides the keyboard |
| bug | LED: keyd target without checking the grab; EV_LED swallowed; NumLock safeguard |
| bug | keyd: `[ids]` limited to 05ac:0256, asymmetric F4 |
| bug | Python: wrong model table, mouse/trackpad taken for keyboards |
| bug | Python `--led`: raw hidraw write, global aggregated state |
| bug | wake-monitor: 0x13 not decoded, active without a 0x13 report |
| feature | Fn, Eject, wake, Lock decoded on D-Bus |
