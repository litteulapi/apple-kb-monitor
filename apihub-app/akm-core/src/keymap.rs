//! Special keys and manual key mapping, without keyd, grab or uinput.

use crate::{tr, N_};
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub use crate::keycodes::KEYCODES;

/// Code of a `KEY_*` name (exact, upper case, as in `input-event-codes.h`).
#[must_use]
pub fn code_of(name: &str) -> Option<u16> {
    KEYCODES.iter().find(|(n, _)| *n == name).map(|&(_, c)| c)
}

/// First `KEY_*` name of a code.
#[must_use]
pub fn name_of(code: u16) -> Option<&'static str> {
    KEYCODES.iter().find(|&&(_, c)| c == code).map(|&(n, _)| n)
}

/// Name or `#<code>` when the code has no name.
pub fn display_code(code: u16) -> String {
    name_of(code).map_or_else(|| format!("#{code}"), str::to_string)
}

/// hwdb spelling of a code: the `KEY_*` name lower-cased without `KEY_`.
#[must_use]
pub fn hwdb_keyname(code: u16) -> Option<String> {
    name_of(code).map(|n| n.trim_start_matches("KEY_").to_ascii_lowercase())
}

/// Code of an hwdb key name (`brightnessdown` → 224).
#[must_use]
pub fn code_of_hwdb_name(s: &str) -> Option<u16> {
    if s.is_empty()
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    code_of(&format!("KEY_{}", s.to_ascii_uppercase()))
}

pub const KEY_FN: u16 = 464;
pub const KEY_EJECTCD: u16 = 161;
pub const KEY_NUMLOCK: u16 = 69;

pub const PAGE_KEYBOARD: u32 = 0x0007_0000;
pub const SC_EJECT: u32 = 0x000c_00b8;
pub const SC_FN: u32 = 0x00ff_0003;

const HID_KEYBOARD: [u16; 256] = [
    0, 0, 0, 0, 30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22,
    47, 17, 45, 21, 44, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 28, 1, 14, 15, 57, 12, 13, 26, 27, 43, 43,
    39, 40, 41, 51, 52, 53, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88, 99, 70, 119, 110,
    102, 104, 111, 107, 109, 106, 105, 108, 103, 69, 98, 55, 74, 78, 96, 79, 80, 81, 75, 76, 77,
    71, 72, 73, 82, 83, 86, 127, 116, 117, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193,
    194, 134, 138, 130, 132, 128, 129, 131, 137, 133, 135, 136, 113, 115, 114, 0, 0, 0, 121, 0, 89,
    93, 124, 92, 94, 95, 0, 0, 0, 122, 123, 90, 91, 85, 0, 0, 0, 0, 0, 0, 0, 111, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 179, 180, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 111, 0, 0, 0, 0, 0, 0, 0,
    29, 42, 56, 125, 97, 54, 100, 126, 164, 166, 165, 163, 161, 115, 114, 113, 150, 158, 159, 128,
    136, 177, 178, 176, 142, 152, 173, 140, 0, 0, 0, 0,
];

/// Kernel default code of a scancode the mapping may touch; `None` = not remappable here.
#[must_use]
pub fn default_code(sc: u32) -> Option<u16> {
    match sc {
        SC_EJECT => Some(KEY_EJECTCD),
        SC_FN => Some(KEY_FN),
        _ if sc & 0xffff_ff00 == PAGE_KEYBOARD => match HID_KEYBOARD[(sc & 0xff) as usize] {
            0 => None,
            c => Some(c),
        },
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysKey {
    /// Name used in `keymap.toml` and on the command line (case-insensitive).
    pub id: &'static str,
    /// What is printed on the key (Apple legend); translated when shown, English in JSON.
    pub legend: &'static str,
    /// HID usage = hwdb scancode.
    pub scancode: u32,
    /// Code `hid-input` gives it before `hid_apple`.
    pub code: u16,
    /// Part of the top row shown by default by `akmctl keys`.
    pub top_row: bool,
}

const fn k(
    id: &'static str,
    legend: &'static str,
    scancode: u32,
    code: u16,
    top_row: bool,
) -> PhysKey {
    PhysKey {
        id,
        legend,
        scancode,
        code,
        top_row,
    }
}

pub const PHYS_KEYS: &[PhysKey] = &[
    k("F1", N_!("Brightness −"), 0x7003a, 59, true),
    k("F2", N_!("Brightness +"), 0x7003b, 60, true),
    k(
        "F3",
        N_!("Expos\u{e9} / Mission Control"),
        0x7003c,
        61,
        true,
    ),
    k("F4", "Dashboard / Launchpad", 0x7003d, 62, true),
    k("F5", N_!("(no icon)"), 0x7003e, 63, true),
    k("F6", N_!("(no icon)"), 0x7003f, 64, true),
    k("F7", N_!("Previous track"), 0x70040, 65, true),
    k("F8", N_!("Play / Pause"), 0x70041, 66, true),
    k("F9", N_!("Next track"), 0x70042, 67, true),
    k("F10", N_!("Mute"), 0x70043, 68, true),
    k("F11", "Volume −", 0x70044, 87, true),
    k("F12", "Volume +", 0x70045, 88, true),
    k("Eject", N_!("Eject ⏏"), SC_EJECT, KEY_EJECTCD, true),
    k("Fn", "fn", SC_FN, KEY_FN, false),
    k("Esc", "esc", 0x70029, 1, false),
    k("Backspace", N_!("Delete ⌫"), 0x7002a, 14, false),
    k("Enter", N_!("Return ↩"), 0x70028, 28, false),
    k("CapsLock", N_!("Caps Lock ⇪"), 0x70039, 58, false),
    k("Grave", "usage 0x35 (` @)", 0x70035, 41, false),
    k("NonUS", "usage 0x64 (< >)", 0x70064, 86, false),
    k("LeftCtrl", N_!("left control"), 0x700e0, 29, false),
    k("LeftShift", N_!("left Shift ⇧"), 0x700e1, 42, false),
    k("LeftAlt", N_!("left Option ⌥"), 0x700e2, 56, false),
    k("LeftCmd", N_!("left Command ⌘"), 0x700e3, 125, false),
    k("RightShift", N_!("right Shift ⇧"), 0x700e5, 54, false),
    k("RightAlt", N_!("right Option ⌥"), 0x700e6, 100, false),
    k("RightCmd", N_!("right Command ⌘"), 0x700e7, 126, false),
    k("Right", "→", 0x7004f, 106, false),
    k("Left", "←", 0x70050, 105, false),
    k("Down", "↓", 0x70051, 108, false),
    k("Up", "↑", 0x70052, 103, false),
];

#[must_use]
pub fn phys_by_id(id: &str) -> Option<&'static PhysKey> {
    PHYS_KEYS.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

#[must_use]
pub fn phys_by_scancode(sc: u32) -> Option<&'static PhysKey> {
    PHYS_KEYS.iter().find(|p| p.scancode == sc)
}

/// Name of a scancode in `keymap.toml`: the physical key id, else `0x…`.
#[must_use]
pub fn scancode_label(sc: u32) -> String {
    phys_by_scancode(sc).map_or_else(|| format!("0x{sc:x}"), |p| p.id.to_string())
}

/// `F1`, `eject`, `0x7003a`, `0x000C00B8` → scancode (whitelisted only).
///
/// # Errors
///
/// [`KeymapError`] when `s` is not a whitelisted key.
pub fn parse_key_ref(s: &str) -> Result<u32, KeymapError> {
    if let Some(p) = phys_by_id(s) {
        return Ok(p.scancode);
    }
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"));
    // 1 to 8 hex digits always fit a u32.
    let parsed = hex
        .filter(|h| (1..=8).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|h| u32::from_str_radix(h, 16).ok());
    let Some(sc) = parsed else {
        // the command between backquotes must stay as is in every catalog (cli.rs test)
        return Err(KeymapError::new(tr!(
                "unknown key {key}: use a key name (F1..F12, Eject, Fn, LeftCmd...; see `akmctl keys --all`) or a HID usage like 0x7003a",
                key = format!("{s:?}")
            )));
    };
    if default_code(sc).is_none() {
        return Err(KeymapError::new(tr!(
            "scancode {code} is not remappable (allowed: keyboard page 0x7xxxx with a kernel default, Eject 0xc00b8, Fn 0xff0003)",
            code = format!("0x{sc:x}")
        )));
    }
    Ok(sc)
}

/// Strict `KEY_*` target: exact upper-case kernel name.
///
/// # Errors
///
/// [`KeymapError`] when `s` is not a whitelisted kernel key name.
pub fn parse_code(s: &str) -> Result<u16, KeymapError> {
    code_of(s).ok_or_else(|| {
        let hint = if code_of(&s.to_ascii_uppercase()).is_some()
            || code_of(&format!("KEY_{}", s.to_ascii_uppercase())).is_some()
        {
            format!(
                " ({})",
                tr!(
                    "did you mean {key}?",
                    key = format!(
                        "KEY_{}",
                        s.trim_start_matches("KEY_")
                            .trim_start_matches("key_")
                            .to_ascii_uppercase()
                    )
                )
            )
        } else {
            String::new()
        };
        KeymapError::new(tr!(
            "unknown key code {code}: expected a KEY_* name of linux/input-event-codes.h{hint}",
            code = format!("{s:?}"),
            hint = hint
        ))
    })
}

/// Aluminium wireless keyboards: `magic_keyboard_alu_fn_keys` in `hid-apple.c`.
pub const ALU_WIRELESS_PIDS: [u16; 9] = [
    0x022c, 0x022d, 0x022e, 0x0239, 0x023a, 0x023b, 0x0255, 0x0256, 0x0257,
];

/// `05ac:0256` → 0x0256 (ALU wireless PIDs only).
///
/// # Errors
///
/// [`KeymapError`] when `s` is not a supported Apple model.
pub fn parse_model(s: &str) -> Result<u16, KeymapError> {
    let bad = || {
        KeymapError::new(tr!("bad model {s}: expected 05ac:PPPP with PPPP an Apple Wireless Keyboard (aluminium) product id", s = format!("{s:?}")))
    };
    let (v, p) = s.split_once(':').ok_or_else(bad)?;
    if !v.eq_ignore_ascii_case("05ac") || p.len() != 4 || !p.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bad());
    }
    let pid = u16::from_str_radix(p, 16).map_err(|_| bad())?;
    if ALU_WIRELESS_PIDS.contains(&pid) {
        Ok(pid)
    } else {
        Err(bad())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FnTrans {
    pub from: u16,
    pub to: u16,
    /// `APPLE_FLAG_FKEY`: the side depends on `fnmode`; otherwise Fn only.
    pub fkey: bool,
}

const fn t(from: u16, to: u16, fkey: bool) -> FnTrans {
    FnTrans { from, to, fkey }
}

/// `magic_keyboard_alu_fn_keys` (hid-apple.c, identical in 6.18 and 7.x).
pub const ALU_FN_KEYS: &[FnTrans] = &[
    t(14, 111, false),  // BACKSPACE → DELETE
    t(28, 110, false),  // ENTER → INSERT
    t(59, 224, true),   // F1 → BRIGHTNESSDOWN
    t(60, 225, true),   // F2 → BRIGHTNESSUP
    t(61, 120, true),   // F3 → SCALE
    t(62, 204, true),   // F4 → DASHBOARD (= KEY_ALL_APPLICATIONS since 5.19)
    t(64, 69, true),    // F6 → NUMLOCK
    t(65, 165, true),   // F7 → PREVIOUSSONG
    t(66, 164, true),   // F8 → PLAYPAUSE
    t(67, 163, true),   // F9 → NEXTSONG
    t(68, 113, true),   // F10 → MUTE
    t(87, 114, true),   // F11 → VOLUMEDOWN
    t(88, 115, true),   // F12 → VOLUMEUP
    t(103, 104, false), // UP → PAGEUP
    t(108, 109, false), // DOWN → PAGEDOWN
    t(105, 102, false), // LEFT → HOME
    t(106, 107, false), // RIGHT → END
];

#[must_use]
pub fn fn_table(pid: u16) -> Option<&'static [FnTrans]> {
    ALU_WIRELESS_PIDS.contains(&pid).then_some(ALU_FN_KEYS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelParam {
    pub name: &'static str,
    pub min: i32,
    pub max: i32,
    pub help: &'static str,
}

/// The parameters of `hid_apple` that `modinfo` lists on the 7.x kernels.
pub const KERNEL_PARAMS: &[KernelParam] = &[
    KernelParam { name: "fnmode", min: 0, max: 4, help: "0 disabled, 1 fkeyslast (media keys first), 2 fkeysfirst, 3 auto (= 1 on an Apple keyboard), 4 fkeysdisabled" },
    KernelParam { name: "iso_layout", min: -1, max: 1, help: "swap ` and < on ISO keyboards: -1 auto, 0 off, 1 on" },
    KernelParam { name: "swap_opt_cmd", min: 0, max: 2, help: "swap Option (Alt) and Command (Meta): 0 Mac, 1 both sides (PC order), 2 left side only" },
    KernelParam { name: "swap_ctrl_cmd", min: 0, max: 1, help: "swap Control and Command: 0 no, 1 yes" },
    KernelParam { name: "swap_fn_leftctrl", min: 0, max: 1, help: "swap Fn and left Control: 0 no, 1 yes" },
];

#[must_use]
pub fn kernel_param(name: &str) -> Option<&'static KernelParam> {
    KERNEL_PARAMS.iter().find(|p| p.name == name)
}

/// Strict decimal: `-1`, `0`, `12`; never `+1`, `01`, ` 1`, `1\n`.
///
/// # Errors
///
/// [`KeymapError`] when `s` is not strict decimal or is out of the parameter's range.
pub fn parse_param_value(p: &KernelParam, s: &str) -> Result<i32, KeymapError> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let ok_shape = !digits.is_empty()
        && digits.len() <= 3
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !(digits.len() > 1 && digits.starts_with('0'))
        && s != "-0";
    let v = if ok_shape {
        s.parse::<i32>().ok()
    } else {
        None
    };
    match v {
        Some(v) if (p.min..=p.max).contains(&v) => Ok(v),
        _ => Err(KeymapError::new(tr!(
            "invalid {name} value {s} (expected {min}..={max}: {help})",
            name = p.name,
            s = format!("{s:?}"),
            min = p.min,
            max = p.max,
            help = p.help
        ))),
    }
}

/// Current values of the parameters (`None` = absent / unreadable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HidState {
    pub fnmode: Option<i32>,
    pub iso_layout: Option<i32>,
    pub swap_opt_cmd: Option<i32>,
    pub swap_ctrl_cmd: Option<i32>,
    pub swap_fn_leftctrl: Option<i32>,
}

impl HidState {
    /// Kernel defaults (module freshly loaded, no option).
    pub const KERNEL_DEFAULT: HidState = HidState {
        fnmode: Some(3),
        iso_layout: Some(-1),
        swap_opt_cmd: Some(0),
        swap_ctrl_cmd: Some(0),
        swap_fn_leftctrl: Some(0),
    };

    #[must_use]
    pub fn read_in(dir: &Path) -> HidState {
        let r = |n: &str| {
            let p = kernel_param(n)?;
            let s = std::fs::read_to_string(dir.join(n)).ok()?;
            parse_param_value(p, s.trim()).ok()
        };
        HidState {
            fnmode: r("fnmode"),
            iso_layout: r("iso_layout"),
            swap_opt_cmd: r("swap_opt_cmd"),
            swap_ctrl_cmd: r("swap_ctrl_cmd"),
            swap_fn_leftctrl: r("swap_fn_leftctrl"),
        }
    }

    #[must_use]
    pub fn read() -> HidState {
        Self::read_in(Path::new(crate::hid_params::SYSFS_DIR))
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<i32> {
        match name {
            "fnmode" => self.fnmode,
            "iso_layout" => self.iso_layout,
            "swap_opt_cmd" => self.swap_opt_cmd,
            "swap_ctrl_cmd" => self.swap_ctrl_cmd,
            "swap_fn_leftctrl" => self.swap_fn_leftctrl,
            _ => None,
        }
    }

    pub fn set(&mut self, name: &str, v: i32) {
        let slot = match name {
            "fnmode" => &mut self.fnmode,
            "iso_layout" => &mut self.iso_layout,
            "swap_opt_cmd" => &mut self.swap_opt_cmd,
            "swap_ctrl_cmd" => &mut self.swap_ctrl_cmd,
            "swap_fn_leftctrl" => &mut self.swap_fn_leftctrl,
            _ => return,
        };
        *slot = Some(v);
    }

    /// `hid_apple` loaded (fnmode readable).
    #[must_use]
    pub fn loaded(&self) -> bool {
        self.fnmode.is_some()
    }
}

fn swap(code: u16, pairs: &[(u16, u16)]) -> u16 {
    pairs
        .iter()
        .find(|(a, _)| *a == code)
        .map_or(code, |&(_, b)| b)
}

/// `hidinput_apple_event` for an Apple keyboard, Fn held or not, `NumLock` LED off.
#[must_use]
pub fn translate(table: Option<&[FnTrans]>, p: &HidState, code: u16, fn_on: bool) -> u16 {
    let mut code = code;
    let fnmode = p.fnmode.unwrap_or(3);
    let real = if fnmode == 3 { 1 } else { fnmode };
    if p.swap_fn_leftctrl.unwrap_or(0) != 0 {
        code = swap(code, &[(KEY_FN, 29), (29, KEY_FN)]);
    }
    if p.iso_layout.unwrap_or(-1) > 0 {
        code = swap(code, &[(41, 86), (86, 41)]);
    }
    match p.swap_opt_cmd.unwrap_or(0) {
        0 => {}
        2 => code = swap(code, &[(56, 125), (125, 56)]),
        _ => code = swap(code, &[(56, 125), (125, 56), (100, 126), (126, 100)]),
    }
    if p.swap_ctrl_cmd.unwrap_or(0) != 0 {
        code = swap(code, &[(29, 125), (125, 29), (97, 126), (126, 97)]);
    }
    if real == 0 {
        return code;
    }
    let Some(tr) = table.and_then(|t| t.iter().find(|t| t.from == code)) else {
        return code;
    };
    let translate = if tr.fkey {
        match real {
            1 => !fn_on,
            2 => fn_on,
            _ => false,
        }
    } else {
        fn_on
    };
    if translate {
        tr.to
    } else {
        code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XkbKey {
    /// Keysym of the default `evdev` xkb rules (`symbols/inet`, `pc`).
    pub keysym: &'static str,
    /// `Qt::Key` value `KWin` hands to `KGlobalAccel` (`action(int)` argument).
    pub qt_key: u32,
    /// Name KDE shows in the shortcut settings.
    pub qt_name: &'static str,
}

const fn x(keysym: &'static str, qt_key: u32, qt_name: &'static str) -> XkbKey {
    XkbKey {
        keysym,
        qt_key,
        qt_name,
    }
}

/// evdev code → keysym → Qt key, for the codes the Apple top row and the Fn layer produce.
#[must_use]
pub fn xkb_of(code: u16) -> Option<XkbKey> {
    Some(match code {
        224 => x(
            "XF86MonBrightnessDown",
            0x0100_00b3,
            "Monitor Brightness Down",
        ),
        225 => x("XF86MonBrightnessUp", 0x0100_00b2, "Monitor Brightness Up"),
        120 => x("XF86LaunchA", 0x0100_00ae, "Launch (C)"),
        204 => x("XF86LaunchB", 0x0100_00af, "Launch (D)"),
        228 => x("XF86KbdLightOnOff", 0x0100_00b4, "Keyboard Light On/Off"),
        229 => x(
            "XF86KbdBrightnessDown",
            0x0100_00b6,
            "Keyboard Brightness Down",
        ),
        230 => x("XF86KbdBrightnessUp", 0x0100_00b5, "Keyboard Brightness Up"),
        165 => x("XF86AudioPrev", 0x0100_0082, "Media Previous"),
        164 => x("XF86AudioPlay", 0x0100_0080, "Media Play"),
        163 => x("XF86AudioNext", 0x0100_0083, "Media Next"),
        113 => x("XF86AudioMute", 0x0100_0071, "Volume Mute"),
        114 => x("XF86AudioLowerVolume", 0x0100_0070, "Volume Down"),
        115 => x("XF86AudioRaiseVolume", 0x0100_0072, "Volume Up"),
        161 => x("XF86Eject", 0x0100_00b9, "Eject"),
        69 => x("Num_Lock", 0x0100_0025, "NumLock"),
        111 => x("Delete", 0x0100_0007, "Del"),
        110 => x("Insert", 0x0100_0006, "Ins"),
        104 => x("Prior", 0x0100_0016, "PgUp"),
        109 => x("Next", 0x0100_0017, "PgDown"),
        102 => x("Home", 0x0100_0010, "Home"),
        107 => x("End", 0x0100_0011, "End"),
        59..=68 => {
            const N: [&str; 10] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10"];
            let i = code - 59;
            x(
                N[usize::from(i)],
                0x0100_0030 + u32::from(i),
                N[usize::from(i)],
            )
        }
        87 => x("F11", 0x0100_003a, "F11"),
        88 => x("F12", 0x0100_003b, "F12"),
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub key: &'static PhysKey,
    /// Code after the hwdb (= `key.code` when not remapped).
    pub base: u16,
    pub remapped: bool,
    /// Code KDE receives without Fn.
    pub plain: u16,
    /// Code KDE receives with Fn held (`None` = not modelled: no Fn table).
    pub with_fn: u16,
}

/// Rows for the keys of `keys` given the parameters and the hwdb overrides (`scancode → code`)
/// currently installed for this PID.
#[must_use]
pub fn effective_table(
    pid: u16,
    p: &HidState,
    overrides: &BTreeMap<u32, u16>,
    keys: &[&'static PhysKey],
) -> Vec<KeyRow> {
    let table = fn_table(pid);
    keys.iter()
        .map(|&key| {
            let base = overrides.get(&key.scancode).copied().unwrap_or(key.code);
            KeyRow {
                key,
                base,
                remapped: base != key.code,
                plain: translate(table, p, base, false),
                with_fn: translate(table, p, base, true),
            }
        })
        .collect()
}

/// Known traps of a row, for display (an `N_!` msgid: translate with `gettext`).
#[must_use]
pub fn row_note(r: &KeyRow, p: &HidState) -> Option<&'static str> {
    if r.plain == KEY_NUMLOCK || r.with_fn == KEY_NUMLOCK {
        return Some(N_!("NumLock: with the NumLock LED on, hid_apple turns J K L U I O 7 8 9 M 0 ; / P - into keypad keys (APPLE_NUMLOCK_EMULATION); press again to leave"));
    }
    if r.key.scancode == SC_FN && r.remapped {
        return Some(N_!("Fn remapped: the Fn layer of hid_apple is lost"));
    }
    if r.remapped
        && fn_table(0x0256).is_some_and(|t| t.iter().any(|t| t.from == r.key.code))
        && r.plain == r.with_fn
    {
        return Some(N_!(
            "remapped before hid_apple: no Fn layer on this key any more"
        ));
    }
    if matches!(r.key.id, "Grave" | "NonUS") && p.iso_layout.unwrap_or(-1) < 0 {
        return Some(N_!("iso_layout=-1: hid_apple swaps these two keys when the keyboard reports the ISO country code"));
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapError(pub String);

impl KeymapError {
    pub fn new(m: impl Into<String>) -> Self {
        Self(m.into())
    }
    fn at(line: usize, m: impl fmt::Display) -> Self {
        Self(tr!("line {line}: {m}", line = line, m = m))
    }
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeymapError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Keys do what their Apple legend says: the behaviour of a fresh install, nothing remapped.
    Apple,
    /// F1..F12 first, media with Fn (fnmode=2).
    FKeys,
    /// PC modifier order Ctrl Meta Alt (`swap_opt_cmd=1`), legend keys kept.
    LinuxPc,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Apple, Preset::FKeys, Preset::LinuxPc];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Preset::Apple => "apple",
            Preset::FKeys => "fkeys",
            Preset::LinuxPc => "linux-pc",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Preset::Apple => N_!("Apple (key legends)"),
            Preset::FKeys => N_!("Classic F1-F12"),
            Preset::LinuxPc => N_!("Linux PC (Cmd↔Alt)"),
        }
    }

    /// Parse a preset name.
    ///
    /// # Errors
    ///
    /// [`KeymapError`] when `s` names no preset.
    pub fn parse(s: &str) -> Result<Preset, KeymapError> {
        Preset::ALL
            .into_iter()
            .find(|p| p.name() == s)
            .ok_or_else(|| {
                KeymapError::new(tr!(
                    "unknown preset {preset} (apple, fkeys, linux-pc)",
                    preset = format!("{s:?}")
                ))
            })
    }

    /// Kernel parameters the preset sets (others are left alone).
    #[must_use]
    pub fn params(self) -> &'static [(&'static str, i32)] {
        match self {
            Preset::Apple => &[("fnmode", 1), ("swap_opt_cmd", 0)],
            Preset::FKeys => &[("fnmode", 2), ("swap_opt_cmd", 0)],
            Preset::LinuxPc => &[("fnmode", 1), ("swap_opt_cmd", 1)],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// `None` (the default): the `hid_apple` parameters are left as they are.
    pub preset: Option<Preset>,
    /// Product ids the key overrides apply to.
    pub models: Vec<u16>,
    /// Explicit parameters, on top of the preset's.
    pub params: BTreeMap<String, i32>,
    /// scancode → evdev code.
    pub keys: BTreeMap<u32, u16>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            preset: None,
            models: vec![0x0256],
            params: BTreeMap::new(),
            keys: BTreeMap::new(),
        }
    }
}

impl Profile {
    /// Preset parameters overlaid with the explicit ones.
    pub fn effective_params(&self) -> BTreeMap<String, i32> {
        let mut m: BTreeMap<String, i32> = self
            .preset
            .map_or(&[][..], Preset::params)
            .iter()
            .map(|&(n, v)| (n.to_string(), v))
            .collect();
        m.extend(self.params.iter().map(|(k, v)| (k.clone(), *v)));
        m
    }

    /// The hwdb records of this profile (empty = nothing to install).
    #[must_use]
    pub fn hwdb_records(&self) -> Vec<HwdbRecord> {
        if self.keys.is_empty() {
            return Vec::new();
        }
        let mut pids = self.models.clone();
        pids.sort_unstable();
        pids.dedup();
        pids.into_iter()
            .map(|pid| HwdbRecord {
                pid,
                keys: self.keys.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub active: String,
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            active: "default".into(),
            profiles: BTreeMap::from([("default".to_string(), Profile::default())]),
        }
    }
}

pub const KEYMAP_REL_PATH: &str = "apple-kb-monitor/keymap.toml";
pub const MAX_FILE: usize = 64 * 1024;

#[must_use]
pub fn valid_profile_name(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `<config home>/apple-kb-monitor/keymap.toml` ([`crate::paths::config_home`]).
#[must_use]
pub fn default_path() -> PathBuf {
    crate::paths::config_home().join(KEYMAP_REL_PATH)
}

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Str(String),
    Int(i64),
    Arr(Vec<String>),
}

fn parse_string(s: &str, line: usize) -> Result<(String, &str), KeymapError> {
    let rest = s
        .strip_prefix('"')
        .ok_or_else(|| KeymapError::at(line, tr!("expected a quoted string")))?;
    let end = rest
        .find('"')
        .ok_or_else(|| KeymapError::at(line, tr!("unterminated string")))?;
    let body = &rest[..end];
    if body.contains('\\') || body.chars().any(char::is_control) {
        return Err(KeymapError::at(
            line,
            tr!("escapes and control characters are not allowed in strings"),
        ));
    }
    Ok((body.to_string(), &rest[end + 1..]))
}

fn end_of_line(rest: &str, line: usize) -> Result<(), KeymapError> {
    let r = rest.trim_start();
    if r.is_empty() || r.starts_with('#') {
        Ok(())
    } else {
        Err(KeymapError::at(
            line,
            tr!("unexpected text {r}", r = format!("{r:?}")),
        ))
    }
}

fn parse_value(s: &str, line: usize) -> Result<Val, KeymapError> {
    let s = s.trim_start();
    if s.starts_with('"') {
        let (v, rest) = parse_string(s, line)?;
        end_of_line(rest, line)?;
        return Ok(Val::Str(v));
    }
    if let Some(mut rest) = s.strip_prefix('[') {
        let mut out = Vec::new();
        loop {
            rest = rest.trim_start();
            if let Some(r) = rest.strip_prefix(']') {
                end_of_line(r, line)?;
                return Ok(Val::Arr(out));
            }
            let (v, r) = parse_string(rest, line)?;
            out.push(v);
            let r = r.trim_start();
            rest = match (r.strip_prefix(','), r.starts_with(']')) {
                (Some(r), _) => r,
                (None, true) => r,
                _ => return Err(KeymapError::at(line, tr!("expected , or ] in array"))),
            };
        }
    }
    let tok_end = s
        .find(|c: char| c.is_whitespace() || c == '#')
        .unwrap_or(s.len());
    let (tok, rest) = s.split_at(tok_end);
    end_of_line(rest, line)?;
    let digits = tok.strip_prefix('-').unwrap_or(tok);
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(KeymapError::at(
            line,
            tr!(
                "bad value {tok} (string, integer or array of strings)",
                tok = format!("{tok:?}")
            ),
        ));
    }
    tok.parse()
        .map(Val::Int)
        .map_err(|_| KeymapError::at(line, tr!("bad integer {tok}", tok = format!("{tok:?}"))))
}

fn parse_key_token(s: &str, line: usize) -> Result<(String, &str), KeymapError> {
    if s.starts_with('"') {
        return parse_string(s, line);
    }
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(s.len());
    if end == 0 {
        return Err(KeymapError::at(line, tr!("expected a key")));
    }
    Ok((s[..end].to_string(), &s[end..]))
}

type RawProfile = (
    Option<Preset>,
    Option<Vec<u16>>,
    BTreeMap<String, i32>,
    BTreeMap<u32, u16>,
);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Section {
    Root,
    Profile(String),
    Keys(String),
    Params(String),
}

fn duplicate_key(key: &str, line: usize) -> KeymapError {
    KeymapError::at(line, tr!("duplicate key {key}", key = format!("{key:?}")))
}

/// `[profile.NAME]`, `[profile.NAME.keys]`, `[profile.NAME.params]` (after the `[`).
fn parse_section_header(h: &str, line: usize) -> Result<(Section, String), KeymapError> {
    let end = h
        .find(']')
        .ok_or_else(|| KeymapError::at(line, tr!("unterminated section header")))?;
    end_of_line(&h[end + 1..], line)?;
    let parts: Vec<&str> = h[..end].trim().split('.').collect();
    let (s, name) = match parts.as_slice() {
        ["profile", n] => (Section::Profile((*n).to_string()), *n),
        ["profile", n, "keys"] => (Section::Keys((*n).to_string()), *n),
        ["profile", n, "params"] => (Section::Params((*n).to_string()), *n),
        _ => {
            return Err(KeymapError::at(
                line,
                tr!(
                    "unknown section [{section}] (profile.NAME, profile.NAME.keys, profile.NAME.params)",
                    section = &h[..end]
                ),
            ))
        }
    };
    if !valid_profile_name(name) {
        return Err(KeymapError::at(
            line,
            tr!(
                "bad profile name {name} (a-z 0-9 - _, 1-32 characters)",
                name = format!("{name:?}")
            ),
        ));
    }
    Ok((s, name.to_string()))
}

fn profile_entry(
    e: &mut RawProfile,
    n: &str,
    key: &str,
    val: Val,
    line: usize,
) -> Result<(), KeymapError> {
    match (key, val) {
        ("preset", _) if e.0.is_some() => return Err(duplicate_key(key, line)),
        ("models", _) if e.1.is_some() => return Err(duplicate_key(key, line)),
        ("preset", Val::Str(s)) => e.0 = Some(Preset::parse(&s).map_err(|er| KeymapError::at(line, er))?),
        ("models", Val::Arr(a)) => {
            if a.is_empty() || a.len() > ALU_WIRELESS_PIDS.len() {
                return Err(KeymapError::at(line, tr!("models: 1 to 9 entries")));
            }
            let mut v = Vec::new();
            for m in &a {
                let pid = parse_model(m).map_err(|er| KeymapError::at(line, er))?;
                if v.contains(&pid) {
                    return Err(KeymapError::at(line, tr!("duplicate model {m}", m = format!("{m:?}"))));
                }
                v.push(pid);
            }
            e.1 = Some(v);
        }
        (k, _) => return Err(KeymapError::at(line, tr!("unknown or ill-typed key {k} in [profile.{n}] (preset = \"apple\", models = [\"05ac:0256\"])", k = format!("{k:?}"), n = n))),
    }
    Ok(())
}

fn params_entry(e: &mut RawProfile, key: &str, val: &Val, line: usize) -> Result<(), KeymapError> {
    let p = kernel_param(key).ok_or_else(|| {
        KeymapError::at(
            line,
            tr!(
                "unknown hid_apple parameter {key} ({known})",
                key = format!("{key:?}"),
                known = KERNEL_PARAMS
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    })?;
    let &Val::Int(v) = val else {
        return Err(KeymapError::at(
            line,
            tr!("{key} must be an integer", key = key),
        ));
    };
    let v = i32::try_from(v)
        .ok()
        .filter(|v| (p.min..=p.max).contains(v))
        .ok_or_else(|| {
            KeymapError::at(
                line,
                tr!(
                    "{key} = {v} out of range {min}..={max} ({help})",
                    key = key,
                    v = v,
                    min = p.min,
                    max = p.max,
                    help = p.help
                ),
            )
        })?;
    if e.2.insert(key.to_string(), v).is_some() {
        return Err(duplicate_key(key, line));
    }
    Ok(())
}

fn keys_entry(e: &mut RawProfile, key: &str, val: Val, line: usize) -> Result<(), KeymapError> {
    let sc = parse_key_ref(key).map_err(|er| KeymapError::at(line, er))?;
    let Val::Str(target) = val else {
        return Err(KeymapError::at(
            line,
            tr!("{key} must be a \"KEY_*\" string", key = key),
        ));
    };
    let code = parse_code(&target).map_err(|er| KeymapError::at(line, er))?;
    if e.3.insert(sc, code).is_some() {
        return Err(KeymapError::at(
            line,
            tr!(
                "key {key} mapped twice (same scancode 0x{sc})",
                key = format!("{key:?}"),
                sc = format!("{sc:x}")
            ),
        ));
    }
    Ok(())
}

impl Keymap {
    /// Strict parse: any unknown section/key, bad value, duplicate or out-of-whitelist entry is an
    /// error (nothing is silently ignored).
    ///
    /// # Errors
    ///
    /// [`KeymapError`] naming the first offending line.
    pub fn parse(src: &str) -> Result<Keymap, KeymapError> {
        if src.len() > MAX_FILE {
            return Err(KeymapError::new(tr!(
                "file larger than {MAX_FILE} bytes",
                MAX_FILE = MAX_FILE
            )));
        }
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let mut sec = Section::Root;
        let mut schema = None;
        let mut active: Option<String> = None;
        let mut profiles: BTreeMap<String, RawProfile> = BTreeMap::new();
        let mut seen_sections = Vec::new();
        for (i, raw) in src.lines().enumerate() {
            let line = i + 1;
            let l = raw.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Some(h) = l.strip_prefix('[') {
                let (s, name) = parse_section_header(h, line)?;
                if seen_sections.contains(&s) {
                    return Err(KeymapError::at(line, tr!("duplicate section")));
                }
                seen_sections.push(s.clone());
                profiles.entry(name).or_default();
                sec = s;
                continue;
            }
            let (key, rest) = parse_key_token(l, line)?;
            let rest = rest
                .trim_start()
                .strip_prefix('=')
                .ok_or_else(|| KeymapError::at(line, tr!("expected =")))?;
            let val = parse_value(rest, line)?;
            match &sec {
                Section::Root => match (key.as_str(), val) {
                    ("schema", _) if schema.is_some() => return Err(duplicate_key(&key, line)),
                    ("active", _) if active.is_some() => return Err(duplicate_key(&key, line)),
                    ("schema", Val::Int(1)) => schema = Some(1),
                    ("schema", Val::Int(n)) => return Err(KeymapError::at(line, tr!("unsupported schema {n} (expected 1)", n = n))),
                    ("active", Val::Str(s)) => {
                        if !valid_profile_name(&s) {
                            return Err(KeymapError::at(line, tr!("bad profile name {s}", s = format!("{s:?}"))));
                        }
                        active = Some(s);
                    }
                    (k, _) => return Err(KeymapError::at(line, tr!("unknown or ill-typed top-level key {k} (schema = 1, active = \"NAME\")", k = format!("{k:?}")))),
                },
                Section::Profile(n) => {
                    profile_entry(profiles.entry(n.clone()).or_default(), n, &key, val, line)?;
                }
                Section::Params(n) => {
                    params_entry(profiles.entry(n.clone()).or_default(), &key, &val, line)?;
                }
                Section::Keys(n) => {
                    keys_entry(profiles.entry(n.clone()).or_default(), &key, val, line)?;
                }
            }
        }
        if schema.is_none() {
            return Err(KeymapError::new(tr!("missing `schema = 1`")));
        }
        if profiles.len() > 16 {
            return Err(KeymapError::new(tr!("at most 16 profiles")));
        }
        let profiles: BTreeMap<String, Profile> = profiles
            .into_iter()
            .map(|(n, (preset, models, params, keys))| {
                let d = Profile::default();
                (
                    n,
                    Profile {
                        preset,
                        models: models.unwrap_or(d.models),
                        params,
                        keys,
                    },
                )
            })
            .collect();
        let active = active.unwrap_or_else(|| "default".into());
        if !(profiles.contains_key(&active) || active == "default" && profiles.is_empty()) {
            return Err(KeymapError::new(tr!(
                "active profile {active} is not defined",
                active = format!("{active:?}")
            )));
        }
        let mut km = Keymap { active, profiles };
        if km.profiles.is_empty() {
            km = Keymap::default();
        }
        Ok(km)
    }

    /// Deterministic TOML; `parse(to_toml(k)) == k`.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut o = String::from(
            "# apple-kb-monitor: manual key mapping (akmctl keymap ...; docs/KEYS.md)\n\
             # Keys: physical name (F1..F12, Eject, Fn, LeftCmd...) or HID usage (0x7003a).\n\
             # Targets: KEY_* names of linux/input-event-codes.h. Nothing changes until `akmctl keymap apply`.\n",
        );
        let _ = writeln!(o, "schema = 1\nactive = \"{}\"", self.active);
        for (name, p) in &self.profiles {
            let _ = writeln!(o, "\n[profile.{name}]");
            if let Some(pr) = p.preset {
                let _ = writeln!(o, "preset = \"{}\"", pr.name());
            }
            let models: Vec<String> = p
                .models
                .iter()
                .map(|m| format!("\"05ac:{m:04x}\""))
                .collect();
            let _ = writeln!(o, "models = [{}]", models.join(", "));
            if !p.params.is_empty() {
                let _ = writeln!(o, "\n[profile.{name}.params]");
                for (k, v) in &p.params {
                    let _ = writeln!(o, "{k} = {v}");
                }
            }
            if !p.keys.is_empty() {
                let _ = writeln!(o, "\n[profile.{name}.keys]");
                for (sc, c) in &p.keys {
                    let _ = writeln!(o, "{} = \"{}\"", scancode_label(*sc), display_code(*c));
                }
            }
        }
        o
    }

    /// The active profile.
    ///
    /// # Panics
    ///
    /// If `active` names no profile; never for a keymap from [`Self::parse`] or `default()`.
    #[must_use]
    pub fn active_profile(&self) -> &Profile {
        self.profiles
            .get(&self.active)
            .expect("parse guarantees the active profile exists")
    }

    /// Profile to edit, created (no preset, nothing remapped) when missing.
    ///
    /// # Errors
    ///
    /// [`KeymapError`] when `name` is invalid or 16 profiles already exist.
    pub fn profile_mut(&mut self, name: &str) -> Result<&mut Profile, KeymapError> {
        if !valid_profile_name(name) {
            return Err(KeymapError::new(tr!(
                "bad profile name {name} (a-z 0-9 - _, 1-32 characters)",
                name = format!("{name:?}")
            )));
        }
        if !self.profiles.contains_key(name) && self.profiles.len() >= 16 {
            return Err(KeymapError::new(tr!("at most 16 profiles")));
        }
        Ok(self.profiles.entry(name.to_string()).or_default())
    }
}

pub const HWDB_PATH: &str = "/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb";
pub const HWDB_HEADER: &str =
    "# Generated by apple-kb-monitor (akmctl keymap apply): do not edit, use `akmctl keymap`.";
pub const MAX_HWDB: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwdbRecord {
    pub pid: u16,
    pub keys: BTreeMap<u32, u16>,
}

/// `evdev:input:b0005v05ACp0256*` (Bluetooth bus; upper-case hex as in the kernel modalias).
#[must_use]
pub fn hwdb_match(pid: u16) -> String {
    format!("evdev:input:b0005v05ACp{pid:04X}*")
}

/// Canonical hwdb file (sorted, one record per PID).
#[must_use]
pub fn render_hwdb(profile: &str, recs: &[HwdbRecord]) -> String {
    let profile = if valid_profile_name(profile) {
        profile
    } else {
        "unknown"
    };
    let mut recs: Vec<&HwdbRecord> = recs.iter().filter(|r| !r.keys.is_empty()).collect();
    recs.sort_by_key(|r| r.pid);
    let mut o = format!("{HWDB_HEADER}\n# profile: {profile}\n");
    for r in recs {
        o.push('\n');
        o.push_str(&hwdb_match(r.pid));
        o.push('\n');
        for (sc, c) in &r.keys {
            // Codes come from KEYCODES (validated): a name always exists.
            let _ = writeln!(
                o,
                " KEYBOARD_KEY_{sc:x}={}",
                hwdb_keyname(*c).unwrap_or_default()
            );
        }
    }
    o
}

/// Size, character set and final newline of a hwdb file.
fn check_hwdb_text(src: &str) -> Result<(), KeymapError> {
    if src.len() > MAX_HWDB {
        return Err(KeymapError::new(tr!(
            "hwdb file larger than {MAX_HWDB} bytes",
            MAX_HWDB = MAX_HWDB
        )));
    }
    if !src.bytes().all(|b| b == b'\n' || (0x20..0x7f).contains(&b)) {
        return Err(KeymapError::new(tr!(
            "hwdb file: only printable ASCII and \\n are allowed (no \\r, tab, NUL, UTF-8)"
        )));
    }
    if !src.ends_with('\n') {
        return Err(KeymapError::new(tr!("hwdb file must end with a newline")));
    }
    Ok(())
}

/// Close the record being read: every PID gets the keys read so far.
fn flush_hwdb_record(
    cur: &mut Vec<u16>,
    keys: &mut BTreeMap<u32, u16>,
    recs: &mut Vec<HwdbRecord>,
    line: usize,
) -> Result<(), KeymapError> {
    if !cur.is_empty() && keys.is_empty() {
        return Err(KeymapError::at(
            line,
            tr!("match line without any KEYBOARD_KEY_ property"),
        ));
    }
    for pid in cur.drain(..) {
        if recs.iter().any(|r| r.pid == pid) {
            return Err(KeymapError::at(
                line,
                tr!("PID {pid} matched twice", pid = format!("{pid:04X}")),
            ));
        }
        recs.push(HwdbRecord {
            pid,
            keys: keys.clone(),
        });
    }
    keys.clear();
    Ok(())
}

/// PID of `XXXX*` (upper-case hex) after `evdev:input:b0005v05ACp`, whitelisted only.
fn hwdb_match_pid(rest: &str) -> Option<u16> {
    rest.strip_suffix('*')
        .filter(|h| {
            h.len() == 4
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        })
        .and_then(|h| u16::from_str_radix(h, 16).ok())
        .filter(|p| ALU_WIRELESS_PIDS.contains(p))
}

/// `<scancode>=<name>` after ` KEYBOARD_KEY_`: whitelisted scancode and known key name.
fn hwdb_property(rest: &str, line: usize) -> Result<(u32, u16), KeymapError> {
    let (sc, name) = rest
        .split_once('=')
        .ok_or_else(|| KeymapError::at(line, tr!("expected KEYBOARD_KEY_<scancode>=<name>")))?;
    let sc_ok = (1..=8).contains(&sc.len())
        && sc
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && !sc.starts_with('0');
    let sc = sc_ok
        .then(|| u32::from_str_radix(sc, 16).ok())
        .flatten()
        .filter(|s| default_code(*s).is_some())
        .ok_or_else(|| {
            KeymapError::at(
                line,
                tr!("scancode not allowed: {sc}", sc = format!("{sc:?}")),
            )
        })?;
    let code = code_of_hwdb_name(name).ok_or_else(|| {
        KeymapError::at(
            line,
            tr!("unknown key name {name}", name = format!("{name:?}")),
        )
    })?;
    Ok((sc, code))
}

/// Whitelist parser of a user-written hwdb file, run by the privileged helper.
///
/// # Errors
///
/// [`KeymapError`] naming the first line outside the whitelist.
pub fn parse_hwdb(src: &str) -> Result<(Option<String>, Vec<HwdbRecord>), KeymapError> {
    check_hwdb_text(src)?;
    let mut lines = src.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l == HWDB_HEADER => {}
        _ => {
            return Err(KeymapError::new(tr!(
                "hwdb file: first line is not the apple-kb-monitor header"
            )))
        }
    }
    let mut profile = None;
    let mut recs: Vec<HwdbRecord> = Vec::new();
    // PIDs of the record being read
    let mut cur: Vec<u16> = Vec::new();
    let mut keys: BTreeMap<u32, u16> = BTreeMap::new();
    let mut prev_was_record_line = false;
    if src.lines().count() > 512 {
        return Err(KeymapError::new(tr!("hwdb file: more than 512 lines")));
    }
    for (i, l) in lines {
        let line = i + 1;
        if l.is_empty() || l.starts_with('#') {
            if let Some(p) = l.strip_prefix("# profile: ") {
                if profile.is_none() && valid_profile_name(p) {
                    profile = Some(p.to_string());
                }
            }
            flush_hwdb_record(&mut cur, &mut keys, &mut recs, line)?;
            prev_was_record_line = false;
            continue;
        }
        if let Some(rest) = l.strip_prefix("evdev:input:b0005v05ACp") {
            let pid = hwdb_match_pid(rest).ok_or_else(|| {
                KeymapError::at(
                    line,
                    tr!("match line not allowed: {l}", l = format!("{l:?}")),
                )
            })?;
            if !keys.is_empty() {
                flush_hwdb_record(&mut cur, &mut keys, &mut recs, line)?;
            }
            if cur.contains(&pid) {
                return Err(KeymapError::at(line, tr!("duplicate match line")));
            }
            cur.push(pid);
            prev_was_record_line = true;
            continue;
        }
        if let Some(rest) = l.strip_prefix(" KEYBOARD_KEY_") {
            if !prev_was_record_line || cur.is_empty() {
                return Err(KeymapError::at(line, tr!("property outside a record")));
            }
            let (sc, code) = hwdb_property(rest, line)?;
            if keys.insert(sc, code).is_some() {
                return Err(KeymapError::at(
                    line,
                    tr!("scancode {sc} set twice", sc = format!("{sc:x}")),
                ));
            }
            continue;
        }
        return Err(KeymapError::at(
            line,
            tr!("line not allowed: {l}", l = format!("{l:?}")),
        ));
    }
    flush_hwdb_record(&mut cur, &mut keys, &mut recs, src.lines().count())?;
    recs.sort_by_key(|r| r.pid);
    Ok((profile, recs))
}

/// What to install when going from `old` (installed) to `new`.
#[must_use]
pub fn transition(old: &[HwdbRecord], new: &[HwdbRecord]) -> Vec<HwdbRecord> {
    let mut by_pid: BTreeMap<u16, BTreeMap<u32, u16>> = BTreeMap::new();
    for r in new {
        by_pid
            .entry(r.pid)
            .or_default()
            .extend(r.keys.iter().map(|(a, b)| (*a, *b)));
    }
    for r in old {
        for (sc, code) in &r.keys {
            let Some(def) = default_code(*sc) else {
                continue;
            };
            if *code == def {
                continue;
            }
            let e = by_pid.entry(r.pid).or_default();
            e.entry(*sc).or_insert(def);
        }
    }
    by_pid
        .into_iter()
        .filter(|(_, k)| !k.is_empty())
        .map(|(pid, keys)| HwdbRecord { pid, keys })
        .collect()
}

/// Overrides that really change something for `pid` (default entries skipped).
#[must_use]
pub fn overrides_for(recs: &[HwdbRecord], pid: u16) -> BTreeMap<u32, u16> {
    recs.iter()
        .filter(|r| r.pid == pid)
        .flat_map(|r| r.keys.iter())
        .filter(|(sc, c)| default_code(**sc) != Some(**c))
        .map(|(a, b)| (*a, *b))
        .collect()
}

/// Installed file (read-only; absent = no override).
///
/// # Errors
///
/// [`KeymapError`] for an I/O error or invalid content.
pub fn read_installed(path: &Path) -> Result<Vec<HwdbRecord>, KeymapError> {
    match std::fs::read_to_string(path) {
        Ok(s) => parse_hwdb(&s)
            .map(|(_, r)| r)
            .map_err(|e| KeymapError::new(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(KeymapError::new(format!("{}: {e}", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(fnmode: i32) -> HidState {
        HidState {
            fnmode: Some(fnmode),
            ..HidState::KERNEL_DEFAULT
        }
    }

    #[test]
    fn keycode_table_basics() {
        assert_eq!(code_of("KEY_F1"), Some(59));
        assert_eq!(code_of("KEY_BRIGHTNESSDOWN"), Some(224));
        assert_eq!(code_of("KEY_FN"), Some(KEY_FN));
        assert_eq!(code_of("key_f1"), None);
        assert_eq!(code_of("KEY_MAX"), None);
        assert_eq!(
            code_of("KEY_DASHBOARD"),
            Some(204),
            "aliases accepted, as udev does"
        );
        assert_eq!(
            name_of(204),
            Some("KEY_ALL_APPLICATIONS"),
            "canonical name first"
        );
        assert_eq!(hwdb_keyname(224).as_deref(), Some("brightnessdown"));
        assert_eq!(code_of_hwdb_name("brightnessdown"), Some(224));
        assert_eq!(code_of_hwdb_name("BrightnessDown"), None);
        assert_eq!(code_of_hwdb_name("f1\n"), None);
        for (n, c) in KEYCODES {
            assert_eq!(
                code_of_hwdb_name(&n[4..].to_ascii_lowercase()),
                code_of(n),
                "{n} {c}"
            );
        }
    }

    #[test]
    fn matches_system_header() {
        let Ok(h) = std::fs::read_to_string("/usr/include/linux/input-event-codes.h") else {
            return;
        };
        let mut n = 0;
        for l in h.lines() {
            let mut w = l.split_whitespace();
            if w.next() != Some("#define") {
                continue;
            }
            let (Some(name), Some(v)) = (w.next(), w.next()) else {
                continue;
            };
            if !name.starts_with("KEY_") || name == "KEY_MAX" {
                continue;
            }
            let v = if let Some(h) = v.strip_prefix("0x") {
                u16::from_str_radix(h, 16).ok()
            } else if v.starts_with("KEY_") {
                code_of(v) // alias of an earlier name (udev resolves them too)
            } else {
                v.parse().ok()
            };
            if let Some(v) = v {
                n += 1;
                assert_eq!(code_of(name), Some(v), "{name}");
            }
        }
        assert!(n >= 500, "{n}");
    }

    #[test]
    fn scancode_whitelist() {
        assert_eq!(parse_key_ref("f1"), Ok(0x7003a));
        assert_eq!(parse_key_ref("Eject"), Ok(SC_EJECT));
        assert_eq!(parse_key_ref("0x7003a"), Ok(0x7003a));
        assert_eq!(parse_key_ref("0x000C00B8"), Ok(SC_EJECT));
        for bad in [
            "0x70000",
            "0x700ff",
            "0xc00b9",
            "0x10001",
            "0x",
            "0x123456789",
            "7003a",
            "F13",
            "",
            "../x",
            "F1\n",
            "0x7003a;",
        ] {
            assert!(parse_key_ref(bad).is_err(), "{bad:?}");
        }
        assert_eq!(default_code(0x70035), Some(41));
        assert_eq!(default_code(0x700e3), Some(125));
        for p in PHYS_KEYS {
            assert_eq!(default_code(p.scancode), Some(p.code), "{}", p.id);
        }
    }

    #[test]
    fn codes_are_strict() {
        assert_eq!(parse_code("KEY_F6"), Ok(64));
        assert!(parse_code("f6").unwrap_err().0.contains("KEY_F6"));
        assert!(parse_code("KEY_F6 ").is_err());
        assert!(parse_code("KEY_NOPE").is_err());
        assert!(parse_code("KEY_MAX").is_err());
    }

    #[test]
    fn param_values_are_strict() {
        let f = kernel_param("fnmode").unwrap();
        assert_eq!(parse_param_value(f, "4"), Ok(4));
        for bad in ["5", "-1", "01", "+1", " 1", "1\n", "", "-", "-0", "1a"] {
            assert!(parse_param_value(f, bad).is_err(), "{bad:?}");
        }
        let iso = kernel_param("iso_layout").unwrap();
        assert_eq!(parse_param_value(iso, "-1"), Ok(-1));
        assert!(parse_param_value(iso, "-2").is_err());
        assert!(kernel_param("rightalt_as_rightctrl").is_none());
        assert!(kernel_param("ejectcd_as_delete").is_none());
    }

    #[test]
    fn effective_fnmode_1_matches_the_kernel_table() {
        let keys: Vec<&PhysKey> = PHYS_KEYS.iter().filter(|p| p.top_row).collect();
        let rows = effective_table(0x0256, &st(1), &BTreeMap::new(), &keys);
        let got: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| {
                (
                    r.key.id,
                    name_of(r.plain).unwrap(),
                    name_of(r.with_fn).unwrap(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("F1", "KEY_BRIGHTNESSDOWN", "KEY_F1"),
                ("F2", "KEY_BRIGHTNESSUP", "KEY_F2"),
                ("F3", "KEY_SCALE", "KEY_F3"),
                ("F4", "KEY_ALL_APPLICATIONS", "KEY_F4"), // = KEY_DASHBOARD (alias)
                ("F5", "KEY_F5", "KEY_F5"),
                ("F6", "KEY_NUMLOCK", "KEY_F6"),
                ("F7", "KEY_PREVIOUSSONG", "KEY_F7"),
                ("F8", "KEY_PLAYPAUSE", "KEY_F8"),
                ("F9", "KEY_NEXTSONG", "KEY_F9"),
                ("F10", "KEY_MUTE", "KEY_F10"),
                ("F11", "KEY_VOLUMEDOWN", "KEY_F11"),
                ("F12", "KEY_VOLUMEUP", "KEY_F12"),
                ("Eject", "KEY_EJECTCD", "KEY_EJECTCD"),
            ]
        );
        assert!(row_note(&rows[5], &st(1)).unwrap().contains("NumLock"));
    }

    #[test]
    fn effective_other_modes() {
        let f1 = phys_by_id("F1").unwrap();
        let r = |p: &HidState| effective_table(0x0256, p, &BTreeMap::new(), &[f1])[0].clone();
        assert_eq!((r(&st(2)).plain, r(&st(2)).with_fn), (59, 224));
        assert_eq!(
            (r(&st(3)).plain, r(&st(3)).with_fn),
            (224, 59),
            "auto = 1 on Apple"
        );
        assert_eq!((r(&st(0)).plain, r(&st(0)).with_fn), (59, 59));
        assert_eq!((r(&st(4)).plain, r(&st(4)).with_fn), (59, 59));
        let bs = phys_by_id("Backspace").unwrap();
        let row = effective_table(0x0256, &st(2), &BTreeMap::new(), &[bs])[0].clone();
        assert_eq!((row.plain, row.with_fn), (14, 111));
        let row = effective_table(0x0267, &st(1), &BTreeMap::new(), &[f1])[0].clone();
        assert_eq!((row.plain, row.with_fn), (59, 59));
    }

    #[test]
    fn swaps_follow_the_kernel_order() {
        let p = HidState {
            swap_opt_cmd: Some(1),
            ..st(1)
        };
        let la = phys_by_id("LeftAlt").unwrap();
        let rc = phys_by_id("RightCmd").unwrap();
        let rows = effective_table(0x0256, &p, &BTreeMap::new(), &[la, rc]);
        assert_eq!((rows[0].plain, rows[1].plain), (125, 100));
        let p2 = HidState {
            swap_opt_cmd: Some(2),
            ..st(1)
        };
        let rows = effective_table(0x0256, &p2, &BTreeMap::new(), &[la, rc]);
        assert_eq!((rows[0].plain, rows[1].plain), (125, 126));
        let p3 = HidState {
            swap_fn_leftctrl: Some(1),
            ..st(1)
        };
        let fnk = phys_by_id("Fn").unwrap();
        assert_eq!(
            effective_table(0x0256, &p3, &BTreeMap::new(), &[fnk])[0].plain,
            29
        );
    }

    #[test]
    fn hwdb_override_is_seen_by_hid_apple() {
        let f6 = phys_by_id("F6").unwrap();
        let ov = BTreeMap::from([(f6.scancode, code_of("KEY_F13").unwrap())]);
        let r = effective_table(0x0256, &st(1), &ov, &[f6])[0].clone();
        assert!(r.remapped);
        assert_eq!((r.plain, r.with_fn), (183, 183));
        assert!(row_note(&r, &st(1)).unwrap().contains("no Fn layer"));
        let ov = BTreeMap::from([(f6.scancode, 59)]);
        let r = effective_table(0x0256, &st(1), &ov, &[f6])[0].clone();
        assert_eq!((r.plain, r.with_fn), (224, 59));
    }

    #[test]
    fn xkb_table_matches_the_system_xkb_data() {
        let Ok(inet) = std::fs::read_to_string("/usr/share/X11/xkb/symbols/inet") else {
            return;
        };
        let Ok(kc) = std::fs::read_to_string("/usr/share/X11/xkb/keycodes/evdev") else {
            return;
        };
        let evdev_section = inet.split("xkb_symbols \"evdev\"").nth(1).unwrap_or("");
        let evdev_section = evdev_section.split("xkb_symbols").next().unwrap_or("");
        for code in [224u16, 225, 120, 204, 228, 229, 230, 165, 163, 161] {
            let xk = xkb_of(code).unwrap();
            let num = code + 8;
            let iname = format!("<I{num}>");
            let names: Vec<String> = std::iter::once(iname.clone())
                .chain(
                    kc.lines()
                        .filter(|l| l.contains("alias") && l.contains(&format!("= {iname}")))
                        .filter_map(|l| {
                            let a = l.find('<')?;
                            let b = l[a..].find('>')? + a;
                            Some(l[a..=b].to_string())
                        }),
                )
                .collect();
            let found = evdev_section.lines().any(|l| {
                let l = l.trim_start();
                !l.starts_with("//")
                    && names.iter().any(|n| l.starts_with(&format!("key {n}")))
                    && l.contains(xk.keysym)
            });
            assert!(
                found,
                "{} ({code}) → {} not in symbols/inet(evdev)",
                display_code(code),
                xk.keysym
            );
        }
    }

    const SAMPLE: &str = "# my mapping\nschema = 1\nactive = \"work\"\n\n[profile.default]\npreset = \"apple\"\n\n\
[profile.work]\npreset = \"linux-pc\"  # PC order\nmodels = [\"05ac:0256\", \"05AC:0255\"]\n\n\
[profile.work.params]\nfnmode = 2\n\n[profile.work.keys]\nF6 = \"KEY_F6\"\n\"0x70039\" = \"KEY_LEFTCTRL\"\nEject = \"KEY_DELETE\"\n";

    #[test]
    fn parses_and_round_trips() {
        let k = Keymap::parse(SAMPLE).unwrap();
        assert_eq!(k.active, "work");
        let w = &k.profiles["work"];
        assert_eq!(w.preset, Some(Preset::LinuxPc));
        assert_eq!(w.models, vec![0x0256, 0x0255]);
        assert_eq!(w.keys.get(&0x70039), Some(&29));
        assert_eq!(w.keys.get(&SC_EJECT), Some(&111));
        assert_eq!(
            w.effective_params(),
            BTreeMap::from([("fnmode".into(), 2), ("swap_opt_cmd".into(), 1)])
        );
        let again = Keymap::parse(&k.to_toml()).unwrap();
        assert_eq!(again, k);
        assert_eq!(again.to_toml(), k.to_toml(), "deterministic");
        assert_eq!(Keymap::parse("schema = 1\n").unwrap(), Keymap::default());
    }

    #[test]
    fn strict_validation() {
        let bad = [
            ("", "schema"),
            ("schema = 2\n", "unsupported schema"),
            ("schema = 1\nfoo = 1\n", "unknown"),
            (
                "schema = 1\n[profile.a]\npreset = \"mac\"\n",
                "unknown preset",
            ),
            (
                "schema = 1\n[profile.a]\npreset = \"apple\"\npreset = \"apple\"\n",
                "duplicate",
            ),
            ("schema = 1\n[profile.A]\n", "bad profile name"),
            (
                "schema = 1\n[profile.a.keys]\nF13 = \"KEY_F1\"\n",
                "unknown key",
            ),
            (
                "schema = 1\n[profile.a.keys]\nF1 = \"KEY_NOPE\"\n",
                "unknown key code",
            ),
            (
                "schema = 1\n[profile.a.keys]\nF1 = \"KEY_F2\"\n\"0x7003a\" = \"KEY_F3\"\n",
                "mapped twice",
            ),
            (
                "schema = 1\n[profile.a.keys]\nF1 = \"KEY_F2\\nKEY_F3\"\n",
                "escapes",
            ),
            ("schema = 1\n[profile.a.keys]\nF1 = 3\n", "must be"),
            (
                "schema = 1\n[profile.a.params]\nfnmode = 9\n",
                "out of range",
            ),
            (
                "schema = 1\n[profile.a.params]\nrightalt_as_rightctrl = 1\n",
                "unknown hid_apple parameter",
            ),
            (
                "schema = 1\n[profile.a]\nmodels = [\"05ac:0267\"]\n",
                "bad model",
            ),
            (
                "schema = 1\n[profile.a]\nmodels = [\"046d:0256\"]\n",
                "bad model",
            ),
            (
                "schema = 1\nactive = \"nope\"\n[profile.a]\n",
                "not defined",
            ),
            (
                "schema = 1\n[profile.a]\n[profile.a]\n",
                "duplicate section",
            ),
            ("schema = 1\n[other]\n", "unknown section"),
            ("schema = 1 x\n", "unexpected"),
            (
                "schema = 1\n[profile.a.keys]\nF1 = \"KEY_F2\" \"x\"\n",
                "unexpected",
            ),
        ];
        for (src, want) in bad {
            let e = Keymap::parse(src).expect_err(src);
            assert!(e.0.contains(want), "{src:?}: {e}");
        }
    }

    #[test]
    fn hwdb_generation_is_deterministic() {
        let k = Keymap::parse(SAMPLE).unwrap();
        let p = k.active_profile();
        let out = render_hwdb("work", &p.hwdb_records());
        assert_eq!(
            out,
            format!(
                "{HWDB_HEADER}\n# profile: work\n\nevdev:input:b0005v05ACp0255*\n KEYBOARD_KEY_70039=leftctrl\n KEYBOARD_KEY_7003f=f6\n KEYBOARD_KEY_c00b8=delete\n\n\
evdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_70039=leftctrl\n KEYBOARD_KEY_7003f=f6\n KEYBOARD_KEY_c00b8=delete\n"
            )
        );
        let (prof, recs) = parse_hwdb(&out).unwrap();
        assert_eq!(prof.as_deref(), Some("work"));
        assert_eq!(render_hwdb("work", &recs), out, "parse ∘ render = id");
        assert!(
            Profile::default().hwdb_records().is_empty(),
            "default = nothing installed"
        );
    }

    #[test]
    fn hwdb_header_bounds_pid_and_read_installed() {
        let hdr = HWDB_HEADER;
        let rec = "evdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n";
        assert!(parse_hwdb(&format!("# not our header\n{rec}")).is_err());
        assert!(
            parse_hwdb(&format!("{hdr} \n{rec}")).is_err(),
            "header + trailing blank"
        );
        assert!(parse_hwdb(&format!("{hdr}\n{rec}")).is_ok());
        for pid in ["00256", "256", "0256 ", "025g", "025a"] {
            let s = format!("{hdr}\nevdev:input:b0005v05ACp{pid}*\n KEYBOARD_KEY_7003f=f6\n");
            assert!(parse_hwdb(&s).is_err(), "{pid:?}");
        }
        let base = format!("{hdr}\n{rec}");
        let fill = |total: usize| {
            let mut s = base.clone();
            while s.len() + 100 <= total {
                let _ = writeln!(s, "#{}", "x".repeat(98));
            }
            let rest = total - s.len();
            if rest > 0 {
                let _ = writeln!(s, "#{}", "y".repeat(rest - 2));
            }
            assert_eq!(s.len(), total);
            s
        };
        assert!(fill(MAX_HWDB).lines().count() <= 512);
        assert!(
            parse_hwdb(&fill(MAX_HWDB)).is_ok(),
            "exactly MAX_HWDB bytes"
        );
        assert!(
            parse_hwdb(&fill(MAX_HWDB + 1)).is_err(),
            "MAX_HWDB + 1 bytes"
        );
        let lines = |n: usize| format!("{base}{}", "#\n".repeat(n - base.lines().count()));
        assert_eq!(lines(512).lines().count(), 512);
        assert!(parse_hwdb(&lines(512)).is_ok(), "512 lines");
        assert!(parse_hwdb(&lines(513)).is_err(), "513 lines");
        let d = std::env::temp_dir().join(format!("akm-keymap-ri-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("90-apple-kb-monitor.hwdb");
        assert_eq!(read_installed(&p).unwrap(), Vec::new());
        std::fs::write(&p, format!("{hdr}\n{rec}")).unwrap();
        let r = read_installed(&p).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].pid, 0x0256);
        assert_eq!(r[0].keys.len(), 1);
        std::fs::write(&p, "garbage\n").unwrap();
        let e = read_installed(&p).unwrap_err().to_string();
        assert!(e.contains("90-apple-kb-monitor.hwdb"), "{e}");
        assert!(
            read_installed(&d).is_err(),
            "a directory is unreadable, not absent"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hwdb_whitelist_refuses_everything_else() {
        let ok = format!("{HWDB_HEADER}\n\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n");
        assert!(parse_hwdb(&ok).is_ok());
        let h = HWDB_HEADER;
        let bad = [
            "evdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n".to_string(),            // no header
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6"),         // no final newline
            format!("{h}\nevdev:input:b0003v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n"),       // USB bus
            format!("{h}\nevdev:input:b0005v046Dp0256*\n KEYBOARD_KEY_7003f=f6\n"),       // other vendor
            format!("{h}\nevdev:input:b0005v05ACp0267*\n KEYBOARD_KEY_7003f=f6\n"),       // other model
            format!("{h}\nevdev:input:b0005v05ACp0256\n KEYBOARD_KEY_7003f=f6\n"),        // no *
            format!("{h}\nevdev:input:b0005v05ACp0256*v*\n KEYBOARD_KEY_7003f=f6\n"),     // glob
            format!("{h}\nevdev:name:*\n KEYBOARD_KEY_7003f=f6\n"),                        // other matcher
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003F=f6\n"),       // upper hex
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_07003f=f6\n"),      // leading 0
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_10001=f6\n"),       // page not allowed
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=nope\n"),     // unknown name
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=F6\n"),       // case
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6 \n"),      // trailing blank
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\r\n"),     // CR
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n EVDEV_ABS_00=1\n"), // other property
            format!("{h}\nevdev:input:b0005v05ACp0256*\n ID_INPUT_KEYBOARD=0\n"),         // other property
            format!("{h}\nevdev:input:b0005v05ACp0256*\n  KEYBOARD_KEY_7003f=f6\n"),      // two spaces
            format!("{h}\nevdev:input:b0005v05ACp0256*\n\tKEYBOARD_KEY_7003f=f6\n"),      // tab
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\nKEYBOARD_KEY_7003e=f5\n"),
            format!("{h}\n KEYBOARD_KEY_7003f=f6\n"),                                      // orphan property
            format!("{h}\nevdev:input:b0005v05ACp0256*\n\n KEYBOARD_KEY_7003f=f6\n"),     // blank inside record
            format!("{h}\nevdev:input:b0005v05ACp0256*\n"),                                // empty record
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n KEYBOARD_KEY_7003f=f5\n"),
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003e=f5\n"),
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\u{0}\n"),  // NUL
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n# ë\n"),  // non-ASCII
            format!("{h}\n../../etc/passwd\n"),
            format!("{h}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\nRUN+=\"/bin/sh\"\n"),
            format!("{h}\n{}", "#\n".repeat(600)),
        ];
        for b in &bad {
            assert!(parse_hwdb(b).is_err(), "accepted: {b:?}");
        }
        assert!(parse_hwdb(&"#".repeat(MAX_HWDB + 1)).is_err());
    }

    #[test]
    fn transition_restores_dropped_keys() {
        let old = vec![HwdbRecord {
            pid: 0x0256,
            keys: BTreeMap::from([(0x7003f, 183), (0x7003e, 63), (SC_EJECT, 111)]),
        }];
        let new = vec![HwdbRecord {
            pid: 0x0256,
            keys: BTreeMap::from([(SC_EJECT, 14)]),
        }];
        let t = transition(&old, &new);
        assert_eq!(
            t,
            vec![HwdbRecord {
                pid: 0x0256,
                keys: BTreeMap::from([(0x7003f, 64), (SC_EJECT, 14)])
            }]
        );
        let back = transition(&old, &[]);
        assert_eq!(
            back[0].keys,
            BTreeMap::from([(0x7003f, 64), (SC_EJECT, 161)])
        );
        assert!(overrides_for(&back, 0x0256).is_empty());
        assert!(
            transition(&[], &[]).is_empty(),
            "{:?}",
            transition(&[], &[])
        );
    }

    #[test]
    fn presets() {
        assert_eq!(Preset::parse("linux-pc"), Ok(Preset::LinuxPc));
        assert!(Preset::parse("Apple").is_err());
        assert!(Profile::default().effective_params().is_empty());
        let apple = Profile {
            preset: Some(Preset::Apple),
            ..Profile::default()
        };
        assert_eq!(
            apple.effective_params(),
            BTreeMap::from([("fnmode".into(), 1), ("swap_opt_cmd".into(), 0)])
        );
        for p in Preset::ALL {
            for (n, v) in p.params() {
                let kp = kernel_param(n).unwrap();
                assert!((kp.min..=kp.max).contains(v));
            }
        }
    }

    #[test]
    fn hidstate_reads_a_fixture() {
        let d = std::env::temp_dir().join(format!("akm-keymap-hs-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("fnmode"), "1\n").unwrap();
        std::fs::write(d.join("iso_layout"), "-1\n").unwrap();
        std::fs::write(d.join("swap_opt_cmd"), "7\n").unwrap();
        let s = HidState::read_in(&d);
        assert_eq!(
            (s.fnmode, s.iso_layout, s.swap_opt_cmd, s.swap_ctrl_cmd),
            (Some(1), Some(-1), None, None)
        );
        assert!(s.loaded());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn hex_key_refs_are_whole_hex_digits() {
        assert_eq!(parse_key_ref("0x7003a").unwrap(), 0x7003a);
        for bad in ["0x+7003a", "0x", "0x1234567890", "0x7003g"] {
            assert!(
                parse_key_ref(bad).unwrap_err().0.contains("unknown key"),
                "{bad}"
            );
        }
    }
}
