# Special keys, KDE and manual mapping

Keyboard: Apple Wireless Keyboard A1314 ISO, `05ac:0256` (aluminium 2011, Bluetooth).
Reference machine: Manjaro, kernel 7.1.13, KDE Plasma 6 (Wayland), `hid_apple fnmode=1`, without keyd.

The **default mapping changes nothing**: the package installs no hwdb file and modifies
no parameter. Presets only apply on request (`akmctl keymap preset <name>` then
`akmctl keymap apply`), never at install time.

## 1. What each key does today

`fnmode=1` (media keys by default, Fn + key = F1…F12). Source: `akmctl keys --check`
(kernel table + KDE bindings read from KGlobalAccel on 2026-10-01, without pressing any key).

| Key | Apple legend | evdev code | Keysym (xkb) | KDE action | State |
|---|---|---|---|---|---|
| F1 | brightness − | `KEY_BRIGHTNESSDOWN` | XF86MonBrightnessDown | PowerDevil "Decrease Screen Brightness" | **[measured by the author]**: the LG 34GK950F display (DDC) dims |
| F2 | brightness + | `KEY_BRIGHTNESSUP` | XF86MonBrightnessUp | PowerDevil "Increase Screen Brightness" | **[measured by the author]** |
| F3 | Exposé / Mission Control | `KEY_SCALE` | XF86LaunchA → Qt "Launch (C)" | KWin `ExposeAll` (present windows, all desktops) | to test |
| F4 | Dashboard / Launchpad | `KEY_ALL_APPLICATIONS` (= `KEY_DASHBOARD`) | XF86LaunchB → Qt "Launch (D)" | **none** (Launch (D) is bound to nothing) | fix: `akmctl keymap kde-apply` |
| F5 | (no pictogram) | `KEY_F5` (no translation in the kernel table) | F5 | none: F5 key for the application | to test |
| F6 | (no pictogram) | `KEY_NUMLOCK` | Num_Lock | none (KWin toggles Num Lock) | **pitfall**, see §4 |
| F7 | previous track | `KEY_PREVIOUSSONG` | XF86AudioPrev | "Media Controller" `previousmedia` | to test |
| F8 | play / pause | `KEY_PLAYPAUSE` | XF86AudioPlay | `playpausemedia` | to test |
| F9 | next track | `KEY_NEXTSONG` | XF86AudioNext | `nextmedia` | to test |
| F10 | mute | `KEY_MUTE` | XF86AudioMute | "Audio Volume" `mute` | to test |
| F11 | volume − | `KEY_VOLUMEDOWN` | XF86AudioLowerVolume | `decrease_volume` | to test |
| F12 | volume + | `KEY_VOLUMEUP` | XF86AudioRaiseVolume | `increase_volume` | to test |
| ⏏ Eject | eject | `KEY_EJECTCD` (usage 0x0C:0xB8) | XF86Eject | **none** | remappable (§5) |

With **Fn**: F1…F12 send `KEY_F1`…`KEY_F12` to the application (except for a global binding: on this
machine Fn+F12 = F12 opens Yakuake). Fn+⌫ = Delete, Fn+↩ = Insert, Fn+↑/↓ = Page Up/Down,
Fn+←/→ = Home/End. Eject does not depend on Fn.

The keyboard backlight keys F5/F6 (`KEY_KBDILLUM*`) do not exist on the A1314 (no
backlight, the kernel's aluminium table does not translate them). If a key produced
`KEY_KBDILLUMUP`, KDE would call PowerDevil "Increase Keyboard Brightness" (binding
present), which would find no backlight: nothing visible. A key **without** a KDE action
is simply passed to the focused application; if that application does not use it, nothing
happens (no error, no OSD).

## 2. The kernel → evdev → KDE chain

1. **HID → evdev code** (`drivers/hid/hid-input.c`, `hid_keyboard[]`): the HID usage becomes a
   `KEY_*` code. The *scancode* seen by udev is the usage: `0x7003a` = F1 … `0x70045` = F12,
   `0xc00b8` = Eject, `0xff0003` = Fn (`apple_input_mapping`).
2. **udev hwdb** (optional): `KEYBOARD_KEY_<scancode>=<name>` replaces that code through
   `EVIOCSKEYCODE` (rule `60-evdev.rules`, builtin `keyboard`). It is the only layer that
   `akmctl keymap` writes.
3. **`hid_apple`** (`drivers/hid/hid-apple.c`, `hidinput_apple_event`): in order
   `swap_fn_leftctrl`, `iso_layout`, `swap_opt_cmd`, `swap_ctrl_cmd`, then the model's Fn layer.
   For PID `0x0256` (`USB_DEVICE_ID_APPLE_ALU_WIRELESS_2011_ISO`, quirks
   `APPLE_NUMLOCK_EMULATION | APPLE_HAS_FN | APPLE_ISO_TILDE_QUIRK`), the table is
   `magic_keyboard_alu_fn_keys`: F1→BRIGHTNESSDOWN, F2→BRIGHTNESSUP, F3→SCALE, F4→DASHBOARD,
   F6→NUMLOCK, F7→PREVIOUSSONG, F8→PLAYPAUSE, F9→NEXTSONG, F10→MUTE, F11→VOLUMEDOWN,
   F12→VOLUMEUP (flag `APPLE_FLAG_FKEY`), and without the flag ⌫→DELETE, ↩→INSERT, ↑→PAGEUP,
   ↓→PAGEDOWN, ←→HOME, →→END. **F5 is not in the table** (always `KEY_F5`).
   Depending on `fnmode` for the `APPLE_FLAG_FKEY` entries:

   | fnmode | without Fn | with Fn |
   |---|---|---|
   | 0 disabled | F1…F12 | F1…F12 (and Fn+⌫ etc. inactive) |
   | 1 fkeyslast (**current**) | media / brightness | F1…F12 |
   | 2 fkeysfirst | F1…F12 | media / brightness |
   | 3 auto | = 1 on an Apple keyboard (= 2 on a "non-Apple" keyboard) | |
   | 4 fkeysdisabled | F1…F12 | F1…F12 (Fn+⌫ etc. active) |

4. **evdev → keysym**: the compositor (KWin, libinput) takes the evdev code + 8 as the xkb keycode;
   `/usr/share/X11/xkb/symbols/inet` (`evdev` section) gives the keysym (`<I232>` →
   XF86MonBrightnessDown, `<I128>` → XF86LaunchA, `<I212>` → XF86LaunchB, `<I169>` → XF86Eject…).
5. **keysym → Qt key → KGlobalAccel**: Qt maps XF86LaunchA → `Key_LaunchC`, XF86LaunchB →
   `Key_LaunchD` (hence KWin's default binding `ExposeAll = Launch (C)`, designed for the
   Exposé key of Apple keyboards). KGlobalAccel triggers the bound action: PowerDevil, KWin, MPRIS…
6. **PowerDevil → display**: `org.kde.ScreenBrightness/display0` = "LG Electronics 34GK950F"
   (external, DDC/CI): PowerDevil already knows how to adjust this display, F1/F2 drive it.

Evidence gathered without reading any keystroke (2026-10-01):

- `modinfo hid_apple` (7.1.13): parameters `fnmode` (0-4), `iso_layout`, `swap_opt_cmd`,
  `swap_ctrl_cmd`, `swap_fn_leftctrl`; `rightalt_as_rightctrl` and `ejectcd_as_delete`
  do not exist in this driver.
- `/sys/class/input/event28/device/capabilities/key` (sysfs, without opening the node): bits 0xE0/0xE1
  (brightness), 0x78 (SCALE), 0xCC (ALL_APPLICATIONS), 0x45 (NUMLOCK), 0xA3-0xA5 (media),
  0x71-0x73 (volume), 0xA1 (EJECTCD), 0x1D0 (FN) present.
- KGlobalAccel `action(i)` (D-Bus, read): MonBrightnessDown/Up → `org_kde_powerdevil`,
  LaunchC → `kwin ExposeAll`, LaunchD → nothing, media → `mediacontrol`, volume → `kmix`,
  Eject → nothing.
- `systemd-hwdb query 'evdev:<modalias of event28>'`: only `KEYBOARD_LED_NUMLOCK=0`
  (`60-keyboard.hwdb`, entry for Apple wireless keyboards): no active remapping.

Conclusion: **nothing to fix in the chain for F1/F2, F3, F7-F12**. The gaps are on the KDE side
(F4, Eject) and the F6 pitfall.

## 3. `akmctl keys`

```text
akmctl keys            # F1-F12 + Eject: code without Fn / with Fn, keysym, KDE action
akmctl keys --check    # + verdict [ok] (KDE action bound) / [app] (for the application) / [--] (nothing)
akmctl keys --all      # all known keys (modifiers, arrows, Fn, ⌫, ↩…)
akmctl keys --json     # for scripts (table + KDE actions)
akmctl keys --live     # live view of presses: HID usage, evdev code, name (§3.1)
```

The table is computed (model of `hid-apple.c` + sysfs parameters + installed hwdb); the KDE action
is asked from KGlobalAccel (`action(i)` of the Qt key). No keystroke is read, no
`event`/`hidraw` node is opened, except by `--live`. System Settings › Apple Keyboard has the same table on its **Keys** page.

### 3.1 `akmctl keys --live`: presses live

```text
$ akmctl keys --live
Live key events, read-only, nothing is recorded. Leave: hold Escape 2 s, or Ctrl-C.
  apple  /dev/input/event5
source  HID usage   evdev  name                     state    key
apple   0007:0064   86     KEY_102ND                down     NonUS
apple   0007:0064   86     KEY_102ND                up       NonUS
```

(Shape of the lines; the values in this example are not a measurement.) One line per press,
repeat and release: the HID usage announced by the kernel (`MSC_SCAN`,
page:usage), the evdev code after hwdb and `hid_apple`, its `KEY_*` name, and the key from the
§1 table when the usage is in it. Unlike the table, this is a **measurement**: what the keyboard and
the kernel really emit.

- **Read-only.** The Apple keyboard's evdev node is opened for reading, never grabbed
  (`EVIOCGRAB`), nothing is sent to the keyboard, no `hidraw` is opened.
- **Nothing is recorded.** Lines go to the terminal, nowhere else (no file, no
  log). Do not start the view while typing a password: it would be shown key by key.
- **Exit**: Escape held 2 s, or Ctrl-C. A short Escape press is a key like any other.
- **Rights**: reading `/dev/input/event*` requires the `input` group. Without it, the command says so and
  reads nothing; it goes through no privileged helper.
- **keyd**: if keyd holds the keyboard (`EVIOCGRAB`), the kernel only delivers the Apple node's events
  to keyd. The view then also reads keyd's virtual keyboard: `keyd` lines, with the evdev
  code keyd emits and without HID usage. To see the HID usages, stop keyd for the duration of the test.

## 4. Known pitfalls

- **F6 = Num Lock.** The `APPLE_NUMLOCK_EMULATION` quirk of `hid_apple` turns, while the
  Num LED is on, J K L U I O 7 8 9 M 0 ; / P - into keypad keys. If letters
  "type digits", press F6 again. Neutralizing F6: `akmctl keymap set F6 KEY_F6`… is **not**
  enough (the kernel translates `KEY_F6` again); a target outside the table is needed, e.g.
  `akmctl keymap set F6 KEY_F19` (F6 becomes F19 with or without Fn).
- **The hwdb acts before `hid_apple`**: a key remapped to a code absent from the Fn table loses
  its Fn layer (it sends the same thing with or without Fn); remapped to `KEY_F1`…`KEY_F12`, it
  inherits the Fn layer of the target key. `akmctl keys` shows it.
- **F13-F18 are not "F13" for xkb**: `<FK13>` = XF86Tools, `<FK14>`-`<FK18>` =
  XF86Launch5-9; `KEY_F19` is the first real F19. For a free shortcut, target `KEY_F19`…
- **Remapping survives deleting the file** (`EVIOCSKEYCODE`) until the keyboard
  reconnects: `akmctl keymap reset`/`rollback` therefore first rewrite the default codes.
- **`hid_apple` parameters are global** to all Apple keyboards of the machine.

## 5. Manual mapping (without keyd, without exclusive grab, without uinput)

File: `~/.config/apple-kb-monitor/keymap.toml` (created by the commands; strict TOML
subset, any error is refused with its line number).

```toml
schema = 1
active = "default"

[profile.default]
preset = "apple"                       # apple | fkeys | linux-pc ; absent = parameters unchanged
models = ["05ac:0256"]                 # PIDs of the targeted aluminium keyboards

[profile.default.params]               # optional, on top of the preset
# fnmode = 1

[profile.default.keys]                 # physical key or HID usage → KEY_* code
Eject = "KEY_DELETE"
F6 = "KEY_F19"
"0x70039" = "KEY_LEFTCTRL"             # Caps Lock → Ctrl
```

Named keys: `F1`…`F12`, `Eject`, `Fn`, `Esc`, `Backspace`, `Enter`, `CapsLock`, `Grave`,
`NonUS`, `LeftCtrl`, `LeftShift`, `LeftAlt` (Option), `LeftCmd`, `RightShift`, `RightAlt`,
`RightCmd`, `Up`, `Down`, `Left`, `Right`; otherwise a whitelisted HID usage (keyboard page
`0x7xxxx` with a default code, `0xc00b8`, `0xff0003`). Targets: `KEY_*` names from
`linux/input-event-codes.h` (the same ones udev accepts).

Presets (the `hid_apple` parameters they set; no key remapped). **No preset by
default**: `apply` touches no parameter (neither `fnmode` nor `swap_opt_cmd`), whatever
their current value.

| Preset | Title | Parameters |
|---|---|---|
| `none` (default) | none | unchanged |
| `apple` | Apple (key legends) | `fnmode=1 swap_opt_cmd=0` |
| `fkeys` | classic F1-F12 | `fnmode=2 swap_opt_cmd=0` |
| `linux-pc` | Linux PC (Cmd↔Alt) | `fnmode=1 swap_opt_cmd=1` (PC order Ctrl Meta Alt) |

The "full Apple" preset (F3 Mission Control, F4 Launchpad) = `apple` + `akmctl keymap kde-apply`.

```text
akmctl keymap show [--hwdb]            # profiles; --hwdb: the file `apply` would install
akmctl keymap set <key> <KEY_…>        # [--profile NAME]
akmctl keymap unset <key>
akmctl keymap preset apple|fkeys|linux-pc|none
akmctl keymap use <profile>
akmctl keymap apply [--dry-run]        # installs (administrator password)
akmctl keymap reset                    # removes the hwdb, kernel mapping restored immediately
akmctl keymap rollback                 # goes back to the previously installed file
akmctl set param <name> <value> [--persist]   # fnmode 0-4, iso_layout -1..1, swap_opt_cmd 0-2, swap_ctrl_cmd 0-1, swap_fn_leftctrl 0-1
akmctl get param [<name>]
```

`apply` computes a plan (empty for the default profile: nothing is requested), writes the hwdb to
`/run/user/<uid>/apple-kb-monitor/keymap.hwdb` and runs `pkexec akm-helper install-keymap install`
(polkit action `com.agenceapi.AppleKbMonitor.install-keymap`); parameters go through
`pkexec akm-helper set-fnmode … --persist` (action `set-fnmode`, `/etc/modprobe.d/hid_apple.conf`).
The helper receives no path: it reads this single file (owner = the caller, one link,
not writable by others, 16 KiB), validates it line by line, rewrites it in canonical
form, keeps the old one as `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb.akm-bak`, writes
`/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` (0644 root:root, atomic rename) then runs
`systemd-hwdb update` and `udevadm trigger --settle --subsystem-match=input --action=change`.

File produced (example):

```text
# Generated by apple-kb-monitor (akmctl keymap apply): do not edit, use `akmctl keymap`.
# profile: default

evdev:input:b0005v05ACp0256*
 KEYBOARD_KEY_7003f=f19
 KEYBOARD_KEY_c00b8=delete
```

Validation done (read-only, nothing in `/etc`): this file placed in a temporary root,
`systemd-hwdb update --root=<tmp> --strict` then `systemd-hwdb query --root=<tmp>
'evdev:<real modalias of event28>'` returns exactly the expected `KEYBOARD_KEY_*`; the system
database stays unchanged (`KEYBOARD_LED_NUMLOCK=0` only). `udevadm test` was **not** run
on the node: it runs the `keyboard` builtin, which opens `/dev/input/event28` and would apply
`EVIOCSKEYCODE` to the real keyboard.

D-Bus (daemon, interface `com.agenceapi.AppleKbMonitor1.Keymap` on
`/com/agenceapi/AppleKbMonitor1`): `KeyTable(b) → s`, `Keymap() → s`, `SetKey(s profile, s key,
s code) → s` (`""` = restore), `SetPreset(s, s) → s`, `UseProfile(s) → s`, `Apply() → s`,
`Reset() → s` (empty on success). The **Keys** page of System Settings › Apple Keyboard uses it.

keyd remains an optional alternative (example in `/usr/share/doc/apple-kb-monitor/examples/keyd/`,
see KEYD.md); nothing here depends on it.

## 6. Missing KDE shortcuts

```text
akmctl keymap kde-apply --dry-run      # what would be added
akmctl keymap kde-apply                # adds
akmctl keymap kde-apply --undo         # removes exactly what was added
```

Only binding provided: **F4 (Launch (D)) → plasmashell "Activate Application Launcher"**
(equivalent of Launchpad). It is only added if Launch (D) is bound to nothing, and it is added
to the action's existing keys (Meta, Alt+F1 remain). No other binding is modified.
Eject deliberately has no imposed action (remap it if needed, §5).

## 7. To test (press each key, tick)

Prerequisite: `akmctl keys --check` (shows the same table, presses nothing). Test setup:
an open browser, an audio player (MPRIS) playing, a text editor.

- [x] **F1**: the LG display brightness drops, Plasma OSD. *(measured by the author)*
- [x] **F2**: brightness goes back up. *(measured by the author)*
- [ ] **F3**: "Present Windows" (all windows, all desktops) opens; F3 again closes it.
- [ ] **F4**: nothing happens (expected today). After `akmctl keymap kde-apply`: the application launcher opens.
- [ ] **F5**: in the browser, the page reloads (F5 key passed on).
- [ ] **F6**: Num Lock turns on (Plasma OSD if enabled); in the editor, `j` types `1`. **Press F6 again** (`j` types `j` again).
- [ ] **F7 / F8 / F9**: previous track / play-pause / next track in the player, media OSD.
- [ ] **F10**: mutes (OSD), F10 again restores the sound.
- [ ] **F11 / F12**: volume − / + (OSD).
- [ ] **⏏ Eject**: nothing happens (no KDE action).
- [ ] **Fn+F1** … **Fn+F12**: the corresponding F key is sent to the application (Fn+F5 reloads, Fn+F11 browser full screen; Fn+F12 opens Yakuake on this machine).
- [ ] **Fn+⌫** deletes to the right; **Fn+↩** = Insert; **Fn+↑/↓** = page up/down; **Fn+←/→** = start/end of line.
- [ ] Manual mapping (optional, reversible): `akmctl keymap set Eject KEY_DELETE && akmctl keymap apply`,
      then ⏏ deletes to the right and `akmctl keys` shows `Eject [hwdb: KEY_DELETE]`;
      `akmctl keymap reset`: ⏏ has no effect again, without reconnecting the keyboard.

## 8. Proof to make: `<>` emits `KEY_102ND`

What the code guarantees, and which is tested without a keyboard
(`iso_layout_decides_which_code_the_two_iso_keys_get`, crate `akmctl`): `iso_layout` is
−1, 0 or 1; at 1 the codes `KEY_GRAVE` (41) and `KEY_102ND` (86) are swapped, in both directions
and for these two keys only; at 0 they are not; at −1 ("auto", kernel default)
`hid_apple` decides from the keyboard's country code, which the model does not guess: `akmctl keys --all`
shows it unswapped and flags it.

What is **not measured**: what the `<>` key of the French ISO keyboard actually emits. The
measurement requires a physical press; it remains to be done by the keyboard's owner. Procedure:

1. Note the setting: `akmctl get param iso_layout` (expected: `-1`).
2. Start the view, either:
   - `akmctl keys --live` (§3.1);
   - or `evtest`: `sudo evtest`, pick the Apple keyboard in the list (`/dev/input/eventN`,
     keyboard name; do not pick `keyd virtual keyboard`). If keyd holds the keyboard, stop it
     for the duration of the test (`sudo systemctl stop keyd`), otherwise `evtest` receives nothing from this node.
3. Press `<>` once (right of left Shift), then `@ #` once (left of `&`/1).
4. Expected result for `<>`:

   ```text
   Event: type 4 (EV_MSC), code 4 (MSC_SCAN), value 7xxxx
   Event: type 1 (EV_KEY), code 86 (KEY_102ND), value 1
   ```

   or, with `akmctl keys --live`, a line `… 86  KEY_102ND  down`. And for `@ #`:
   `code 41 (KEY_GRAVE)`.
5. Note both `MSC_SCAN` values (HID usage: `70064` or `70035`): they tell whether the keyboard
   announces its two keys in PC order or in Apple order, hence whether the swap took place.
6. If `<>` gives `KEY_GRAVE` (and `@ #` gives `KEY_102ND`): the automatic swap does not apply
   to this keyboard. Force it: `akmctl set param iso_layout 1` (add `--persist` to keep it),
   redo step 3, then check in an editor that `<` and `>` come out correctly.
7. Record the result (the four lines, the kernel `uname -r`, the value of `iso_layout`) in
   an issue, and tick below.

- [ ] `<>` → `KEY_102ND` with `iso_layout = -1` *(to be measured by the author)*
- [ ] `@ #` → `KEY_GRAVE` with `iso_layout = -1` *(to be measured by the author)*
