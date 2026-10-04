//! Register map of the Apple A1314 HID reports, as a declarative table.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::conv::hex_compact as hex;
use crate::tr;

/// Direction of a HID report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Feature,
    Input,
    Output,
}

impl Direction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feature => "Feature",
            Self::Input => "Input",
            Self::Output => "Output",
        }
    }
}

/// What the code may do with a report (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Safety {
    SafeRead,
    OncePerConnection,
    /// Input report requested (GET Input) once per battery read, right after `0x47`, as Apple's
    /// `getBatteryState` does (R2); also pushed by the keyboard and listened to passively.
    SafeReadInput,
    PassiveInput,
    ManualOnly,
    NeverRead,
    /// Written by Apple's own software, with the same bytes and in the same context, by one named
    /// operation only ([`WriteOp`], [`check_write_op`]).
    WriteApple,
    NeverWrite,
    Unknown,
}

impl Safety {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SafeRead => "SafeRead",
            Self::OncePerConnection => "OncePerConnection",
            Self::SafeReadInput => "SafeReadInput",
            Self::PassiveInput => "PassiveInput",
            Self::ManualOnly => "ManualOnly",
            Self::NeverRead => "NeverRead",
            Self::WriteApple => "WriteApple",
            Self::NeverWrite => "NeverWrite",
            Self::Unknown => "Unknown",
        }
    }

    /// May the daemon request this Feature report on the hardware (`GET_REPORT`)?
    #[must_use]
    pub fn daemon_may_read(self) -> bool {
        matches!(self, Self::SafeRead | Self::OncePerConnection)
    }

    /// May the daemon request this Input report (`GET_REPORT`, type Input)?
    #[must_use]
    pub fn daemon_may_read_input(self) -> bool {
        self == Self::SafeReadInput
    }
}

/// Level of proof of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof {
    Measured,
    Plist,
    Disassembly,
    Source,
    Hypothesis,
}

impl Proof {
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Measured => "[measured]",
            Self::Plist => "[plist]",
            Self::Disassembly => "[disassembly]",
            Self::Source => "[source]",
            Self::Hypothesis => "[hypothesis]",
        }
    }
}

/// Byte order of the multi-byte fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    None,
    Little,
    Big,
    /// Both conventions in one report (0x4C: id byte, then a reversed host address).
    Mixed,
}

impl Endian {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "-",
            Self::Little => "LE",
            Self::Big => "BE",
            Self::Mixed => "mixed",
        }
    }
}

/// How the payload (bytes after the report id) is turned into a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoder {
    /// Not decoded: shown as hex (or not at all for secrets).
    Raw,
    U8,
    /// 0..=100 percentage.
    Percent,
    /// u16 little-endian, millivolts.
    MilliVoltsLe,
    /// u16 big-endian, millivolts.
    MilliVoltsBe,
    /// u16 little-endian shown as `0x0050`.
    VersionLe,
    /// u16 big-endian, plain number.
    U16Be,
    /// Four u16 big-endian: Full / Low / Critical / Empty (mV).
    Thresholds,
    /// ASCII fragment, cut at the first NUL.
    Ascii,
    /// `0x30`: 0 normal, 1 low, 2 / 3 critical.
    BatteryState,
    /// `0x4C`: only the paired host address is understood; never displayed.
    HostAddress,
}

/// One report of the register map.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub id: u8,
    pub dir: Direction,
    /// Total length returned by `GET_REPORT`, id included (`None` = n/a).
    pub len: Option<u8>,
    /// Name used by Apple's drivers, when known.
    pub apple_name: Option<&'static str>,
    /// Our short name.
    pub name: &'static str,
    pub meaning: &'static str,
    pub unit: &'static str,
    pub endian: Endian,
    pub decoder: Decoder,
    pub proof: Proof,
    pub safety: Safety,
    /// Where the information comes from (document and section).
    pub source: &'static str,
}

#[allow(clippy::too_many_arguments)] // reason: one positional row of the register table
const fn r(
    id: u8,
    dir: Direction,
    len: Option<u8>,
    apple_name: Option<&'static str>,
    name: &'static str,
    meaning: &'static str,
    unit: &'static str,
    endian: Endian,
    decoder: Decoder,
    proof: Proof,
    safety: Safety,
    source: &'static str,
) -> Entry {
    Entry {
        id,
        dir,
        len,
        apple_name,
        name,
        meaning,
        unit,
        endian,
        decoder,
        proof,
        safety,
        source,
    }
}

use Decoder as D;
use Direction::{Feature as F, Input as I, Output as O};
use Endian as E;
use Proof as P;
use Safety as S;

/// The register map. Order: safe reads, once-per-connection, passive inputs, then the rest by id.
pub const TABLE: &[Entry] = &[
    r(0x47, F, Some(2), Some("BatteryPercent"), "battery_percent",
      "Battery strength computed by the firmware (declared Input, read as Feature by the kernel)",
      "%", E::None, D::Percent, P::Measured, S::SafeRead, "HARDWARE-HID-REPORTS §1-2"),
    r(0x46, F, Some(3), None, "battery_voltage_instant",
      "Instantaneous battery voltage of the cell pair (same quantity as 0xFF)",
      "mV", E::Little, D::MilliVoltsLe, P::Measured, S::SafeRead, "HARDWARE-HID-REPORTS §3"),
    r(0x49, F, Some(3), Some("BatteryVoltage"), "battery_voltage_latched",
      "Latched battery voltage (\"MVLT\" of Lion), slow, noiseless",
      "mV", E::Little, D::MilliVoltsLe, P::Plist, S::SafeRead, "maintainer notes"),
    r(0x4F, F, Some(3), None, "firmware_version",
      "Firmware version (= bcdDevice of the modalias)",
      "version", E::Little, D::VersionLe, P::Measured, S::OncePerConnection, "HARDWARE-HID-REPORTS §2"),
    r(0x60, F, Some(9), Some("CalibratedBatteryThresholds3"), "battery_thresholds",
      "Battery thresholds Full / Low / Critical / Empty (4 x u16)",
      "mV", E::Big, D::Thresholds, P::Plist, S::OncePerConnection, "maintainer notes"),
    r(0x51, F, Some(9), Some("DeviceName1"), "device_name_1",
      "Keyboard name, fragment 1 of 4 (8 ASCII bytes)", "text", E::None, D::Ascii, P::Measured,
      S::OncePerConnection, "maintainer notes"),
    r(0x52, F, Some(9), Some("DeviceName2"), "device_name_2",
      "Keyboard name, fragment 2 of 4", "text", E::None, D::Ascii, P::Measured,
      S::OncePerConnection, "maintainer notes"),
    r(0x53, F, Some(9), Some("DeviceName3"), "device_name_3",
      "Keyboard name, fragment 3 of 4", "text", E::None, D::Ascii, P::Measured,
      S::OncePerConnection, "maintainer notes"),
    r(0x54, F, Some(9), Some("DeviceName4"), "device_name_4",
      "Keyboard name, fragment 4 of 4 (empty for short names; name <= 32 bytes)", "text", E::None,
      D::Ascii, P::Measured, S::OncePerConnection, "maintainer notes"),
    r(0x30, I, Some(2), Some("BatteryState"), "battery_state",
      "Battery state: 0 normal, 1 low, 2-3 critical; pushed by the keyboard and read by GET Input right after 0x47 (Apple R2, getBatteryState; GET answered `30 00` in the exhaustive pass without incident)",
      "enum", E::None, D::BatteryState, P::Disassembly, S::SafeReadInput, "maintainer notes"),
    r(0x04, I, Some(2), None, "sleep",
      "Sleep notification (analogy with the Broadcom reference firmware, unknown to macOS)",
      "-", E::None, D::U8, P::Hypothesis, S::PassiveInput, "maintainer notes"),
    r(0x05, I, Some(2), None, "func_lock",
      "Fn-lock state byte (analogy with the Broadcom reference firmware, unknown to macOS)",
      "-", E::None, D::U8, P::Hypothesis, S::PassiveInput, "maintainer notes"),
    r(0x13, I, Some(2), None, "wake",
      "Bit 0 device ready, bit 1 connection request (a keyboard that is switched off reports bit 1 = 0)",
      "bits", E::None, D::U8, P::Disassembly, S::PassiveInput, "maintainer notes"),
    r(0x11, I, Some(2), None, "eject_fn",
      "Consumer Eject and vendor Fn key", "bits", E::None, D::U8, P::Measured, S::PassiveInput,
      "HARDWARE-HID-REPORTS §1"),
    r(0x12, I, Some(2), None, "media_keys",
      "Play/Pause, Fast Forward, Rewind, Next, Previous", "bits", E::None, D::U8, P::Measured,
      S::PassiveInput, "HARDWARE-HID-REPORTS §1"),
    r(0x4C, F, Some(20), None, "pairing_record",
      "Byte 1 = 0x03, bytes 2-7 = paired host address (reversed), then 12 secret bytes (probable link-key material)",
      "-", E::Mixed, D::HostAddress, P::Measured, S::NeverRead, "HARDWARE-HID-REPORTS §2bis"),
    r(0xFE, F, Some(9), None, "freezes_firmware",
      "Answer box; reading it froze the firmware twice", "-", E::None, D::Raw, P::Measured,
      S::NeverRead, "maintainer notes"),
    r(0x01, I, Some(9), None, "boot_keyboard",
      "Boot keyboard report: key codes (never read nor logged, passive keylogging risk)", "-",
      E::None, D::Raw, P::Source, S::NeverRead, "HARDWARE-HID-REPORTS §1"),
    r(0x34, F, None, None, "magic_kb_address",
      "Magic Keyboard only (CVE-2024-0230): address and name; refused by the A1314", "-", E::None,
      D::Raw, P::Source, S::NeverRead, "maintainer notes"),
    r(0x35, F, None, None, "magic_kb_link_key",
      "Magic Keyboard only (CVE-2024-0230): link key; refused by the A1314", "-", E::None,
      D::Raw, P::Source, S::NeverRead, "maintainer notes"),
    r(0x40, F, None, Some("WillShutdown"), "will_shutdown",
      "Command: the host is going to shut down (write-only; sent by macOS at each shutdown or restart, id only: wire `53 40`)",
      "-", E::None, D::Raw, P::Disassembly, S::WriteApple,
      "maintainer notes"),
    r(0x55, F, Some(65), Some("LongDeviceName"), "long_device_name",
      "Long name, 64 bytes (report of 65: id + UTF-8 name + 0x00 padding), write-only on this keyboard (GET refused 0x03); the one frame Lion's setDeviceName: sends (operation DeviceName; write behind three locks: config, control MTU >= 66, typed confirmation)",
      "text", E::None, D::Raw, P::Disassembly, S::WriteApple,
      "RENAME-KEYBOARD"),
    r(0x41, F, None, Some("RecantConnection"), "recant_connection",
      "Command: give up the connection (Apple's virtual cable unplug), id only (wire `53 41`); sent by macOS 26.5 bluetoothd when the user forgets a connected keyboard (operation Forget, akmctl repair only); effect on the keyboard not measured",
      "-", E::None, D::Raw, P::Disassembly, S::WriteApple,
      "RECONNECTION-PAIRING"),
    r(0x01, O, Some(2), None, "led_output",
      "Keyboard LEDs (Caps Lock...): handled by the kernel through evdev, never by us", "bits",
      E::None, D::Raw, P::Disassembly, S::NeverWrite, "maintainer notes"),
    r(0x43, F, Some(2), Some("UserMode"), "user_mode",
      "Declared by Apple (1-3); absent from the 0x0050 firmware (ERR_INVALID_REPORT_ID)", "enum",
      E::None, D::Raw, P::Measured, S::NeverWrite, "maintainer notes"),
    r(0x44, F, None, Some("FullFactoryDefault"), "full_factory_default",
      "Command: forget ALL link keys (Lion's \"remove keyboard\"); re-pairing from every host", "-",
      E::None, D::Raw, P::Disassembly, S::NeverWrite, "maintainer notes (written only with the manager's agreement)"),
    r(0x45, F, None, Some("FactoryDefault"), "factory_default",
      "Command: factory reset, exact effect unknown; no Apple software calls it", "-", E::None,
      D::Raw, P::Disassembly, S::NeverWrite, "maintainer notes"),
    r(0x4A, F, Some(2), None, "sco_link_state",
      "Written by Apple (3 / 4 = SCO link active / inactive); read value 0x12 is not in that enumeration",
      "enum", E::None, D::U8, P::Disassembly, S::NeverWrite, "maintainer notes (written only with the manager's agreement)"),
    r(0x50, F, None, Some("DeviceNameChange"), "device_name_change",
      "Command: validate a name change (4 fragments)", "-", E::None, D::Raw, P::Disassembly,
      S::NeverWrite, "maintainer notes"),
    r(0xD0, F, None, None, "d0", "Unknown write-only register (maintenance range of the updater)", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "maintainer notes"),
    r(0xD4, F, None, None, "d4", "Unknown write-only register (maintenance range of the updater)", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "maintainer notes"),
    r(0xD5, F, None, None, "d5", "Start / stop of the radio PER test of old Apple HID devices (CoreBluetooth CBHIDPerformanceMonitor, macOS 26.5: SET `D5 07` starts, `D5 00` stops; no shipped client; meaning of 07 unknown); write-only, never written without the manager's agreement", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "maintainer notes"),
    r(0xD1, F, Some(2), None, "d1", "Unknown, constant 0", "-", E::None, D::U8, P::Measured,
      S::NeverWrite, "HARDWARE-HID-REPORTS §2"),
    r(0xD8, F, Some(2), None, "d8", "Unknown, constant 0", "-", E::None, D::U8, P::Measured,
      S::NeverWrite, "HARDWARE-HID-REPORTS §2"),
    r(0xFA, F, None, None, "fa", "Unknown write-only register, unknown to every Apple software examined", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "maintainer notes"),
    r(0xFB, F, None, None, "fb", "Unknown write-only register, unknown to every Apple software examined", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "maintainer notes"),
    r(0xF6, F, Some(3), None, "f6", "Unknown, constant 4", "-", E::Big, D::U16Be, P::Measured,
      S::NeverWrite, "HARDWARE-HID-REPORTS §2"),
    r(0xF7, F, Some(3), None, "f7", "Unknown, constant 4", "-", E::Big, D::U16Be, P::Measured,
      S::NeverWrite, "HARDWARE-HID-REPORTS §2"),
    r(0xC6, F, None, None, "connection_interval_update",
      "Apple: connection interval update (5-byte SET); absent from this keyboard", "-", E::None,
      D::Raw, P::Disassembly, S::NeverWrite, "maintainer notes"),
    r(0xDC, F, None, None, "llr",
      "Apple: low latency / reliable radio setting (3-byte SET); absent from this keyboard", "-",
      E::None, D::Raw, P::Disassembly, S::NeverWrite, "maintainer notes"),
    r(0xD7, F, None, Some("SuperMode"), "super_mode",
      "Magic Mouse / Trackpad multitouch switch; refused by the keyboard", "-", E::None, D::Raw,
      P::Source, S::NeverWrite, "maintainer notes"),
    r(0xB0, F, None, None, "backlight_set",
      "Magic Keyboard backlight; absent from the A1314 (no backlight)", "-", E::None, D::Raw,
      P::Source, S::NeverWrite, "maintainer notes"),
    r(0xBF, F, None, None, "backlight_config",
      "Magic Keyboard backlight configuration; absent from the A1314", "-", E::None, D::Raw,
      P::Source, S::NeverWrite, "maintainer notes"),
    r(0x0A, F, None, None, "usb_bootloader",
      "Cypress USB bootloader report of the wired A1243 (Chen 2009); absent from the A1314", "-",
      E::None, D::Raw, P::Source, S::NeverWrite, "maintainer notes"),
    r(0xFF, F, Some(4), None, "battery_voltage_be",
      "Same voltage as 0x46 (big-endian) followed by 0x01", "mV", E::Big, D::MilliVoltsBe,
      P::Measured, S::ManualOnly, "HARDWARE-HID-REPORTS §3"),
    r(0x5A, F, Some(9), None, "thresholds_copy_5a",
      "Copy of 0x60 (4 x u16)", "mV", E::Big, D::Thresholds, P::Measured, S::ManualOnly,
      "HARDWARE-HID-REPORTS §4"),
    r(0xEB, F, Some(9), None, "thresholds_copy_eb",
      "Copy of 0x60 (second set?)", "mV", E::Big, D::Thresholds, P::Measured, S::ManualOnly,
      "maintainer notes"),
    r(0x5B, F, Some(9), None, "f4_f5_concat", "0xF4 followed by 0xF5 and four zero bytes", "-",
      E::Big, D::Raw, P::Measured, S::ManualOnly, "HARDWARE-HID-REPORTS §2"),
    r(0xF4, F, Some(3), None, "f4", "Constant 1740 (cut-off voltage? radio slots?)", "-", E::Big,
      D::U16Be, P::Hypothesis, S::ManualOnly, "HARDWARE-HID-REPORTS §4"),
    r(0xF5, F, Some(3), None, "f5",
      "Constant 900 (idle delay in seconds?). NOT a voltage", "-", E::Big, D::U16Be,
      P::Hypothesis, S::ManualOnly, "HARDWARE-HID-REPORTS §3-4"),
    r(0xEA, F, Some(2), None, "ea",
      "Second percentage estimator? An isolated 0 must be ignored", "%", E::None, D::U8,
      P::Hypothesis, S::ManualOnly, "HARDWARE-HID-REPORTS §4-5"),
    r(0x09, F, Some(4), None, "caps_lock_delay_flag",
      "Only declared Feature (FF01:0B): flag of the firmware Caps Lock delay (01 = off)", "flag",
      E::None, D::U8, P::Disassembly, S::ManualOnly, "maintainer notes"),
    r(0x5C, F, Some(9), None, "empty_5c", "Always empty", "-", E::None, D::Raw, P::Measured,
      S::ManualOnly, "HARDWARE-HID-REPORTS §2"),
    r(0x5D, F, Some(9), None, "empty_5d", "Always empty", "-", E::None, D::Raw, P::Measured,
      S::ManualOnly, "HARDWARE-HID-REPORTS §2"),
    r(0x4B, F, Some(3), None, "unknown_4b", "Unknown, constant 00 08", "-", E::None, D::Raw,
      P::Measured, S::Unknown, "HARDWARE-HID-REPORTS §2"),
    r(0x4E, F, Some(11), None, "connection_counts",
      "Apple: connection counters (two u32); absent from this keyboard", "-", E::None, D::Raw,
      P::Disassembly, S::Unknown, "maintainer notes"),
    r(0x90, I, None, None, "magic_power",
      "Battery of the Magic Keyboard / Mouse 2 (Power page); absent from the A1314", "%", E::None,
      D::Raw, P::Source, S::Unknown, "maintainer notes"),
];

const fn count(class: Safety, dir: Direction) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < TABLE.len() {
        if TABLE[i].safety as u8 == class as u8 && TABLE[i].dir as u8 == dir as u8 {
            n += 1;
        }
        i += 1;
    }
    n
}

const fn ids<const N: usize>(class: Safety, dir: Direction) -> [u8; N] {
    let mut out = [0u8; N];
    let mut n = 0;
    let mut i = 0;
    while i < TABLE.len() {
        if TABLE[i].safety as u8 == class as u8 && TABLE[i].dir as u8 == dir as u8 {
            out[n] = TABLE[i].id;
            n += 1;
        }
        i += 1;
    }
    out
}

const N_SAFE: usize = count(Safety::SafeRead, Direction::Feature);
#[cfg(test)]
const N_ONCE: usize = count(Safety::OncePerConnection, Direction::Feature);
const N_SAFE_INPUT: usize = count(Safety::SafeReadInput, Direction::Input);

/// Input ids that may be requested (GET Input) once per battery read, right after `0x47`, generated
/// from [`TABLE`]: `0x30` and nothing else.
pub const SAFE_READ_INPUT_IDS: [u8; N_SAFE_INPUT] = ids(Safety::SafeReadInput, Direction::Input);

/// Feature ids that may be requested in routine, in table order, generated from [`TABLE`].
pub const SAFE_READ_IDS: [u8; N_SAFE] = ids(Safety::SafeRead, Direction::Feature);
/// Feature ids that may be requested once per connection, generated from [`TABLE`] (test oracle).
#[cfg(test)]
pub const ONCE_PER_CONNECTION_IDS: [u8; N_ONCE] =
    ids(Safety::OncePerConnection, Direction::Feature);
/// Of those, the ones the daemon actually requests: firmware version and battery thresholds.
pub const DAEMON_ONCE_IDS: [u8; 2] = [0x4F, 0x60];
/// Then, with a lower priority: the name stored in the keyboard `0x51`-`0x54`.
pub const DAEMON_DEFERRED_ONCE_IDS: [u8; 4] = [0x51, 0x52, 0x53, 0x54];

#[must_use]
pub fn lookup(id: u8, dir: Direction) -> Option<&'static Entry> {
    TABLE.iter().find(|e| e.id == id && e.dir == dir)
}

#[must_use]
pub fn lookup_feature(id: u8) -> Option<&'static Entry> {
    lookup(id, Direction::Feature)
}

/// Safety class of any Feature id.
#[must_use]
pub fn classify_feature(id: u8) -> Safety {
    match lookup_feature(id) {
        Some(e) => e.safety,
        None if id >> 4 == 0xD || id >> 4 == 0xF => Safety::NeverWrite,
        None => Safety::Unknown,
    }
}

/// A request refused by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub id: u8,
    pub class: Safety,
    pub write: bool,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = if self.write { "write" } else { "read" };
        write!(
            f,
            "{what} of report {:#04x} refused by the register map (class {})",
            self.id,
            self.class.as_str()
        )
    }
}

impl std::error::Error for Refusal {}

impl From<Refusal> for std::io::Error {
    fn from(r: Refusal) -> Self {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, r)
    }
}

/// May the daemon request this Feature report?
///
/// # Errors
///
/// [`Refusal`] when the report is not readable.
pub fn check_read(id: u8) -> Result<Safety, Refusal> {
    let class = classify_feature(id);
    if class.daemon_may_read() {
        Ok(class)
    } else {
        Err(Refusal {
            id,
            class,
            write: false,
        })
    }
}

/// Safety class of an Input id: its entry's class, else `Unknown`.
#[must_use]
pub fn classify_input(id: u8) -> Safety {
    lookup(id, Direction::Input).map_or(Safety::Unknown, |e| e.safety)
}

/// May the daemon request this Input report (GET Input)?
///
/// # Errors
///
/// [`Refusal`] when the report is not readable.
pub fn check_read_input(id: u8) -> Result<Safety, Refusal> {
    let class = classify_input(id);
    if class.daemon_may_read_input() {
        Ok(class)
    } else {
        Err(Refusal {
            id,
            class,
            write: false,
        })
    }
}

/// A named write operation of Apple's software, with the only ids it may write and the exact length
/// of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteOp {
    /// `WillShutdown`: macOS 26.5 kernel driver, at every shutdown or restart \[disassembly\].
    Shutdown,
    /// `LongDeviceName` (`0x55`, 64 data bytes).
    DeviceName,
    /// `RecantConnection` (`0x41`, id only).
    Forget,
}

impl WriteOp {
    /// Every named operation (the sweep tests iterate this list).
    pub const ALL: [WriteOp; 3] = [Self::Shutdown, Self::DeviceName, Self::Forget];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shutdown => "Shutdown",
            Self::DeviceName => "DeviceName",
            Self::Forget => "Forget",
        }
    }

    /// One translated line for `akmctl info`'s legend.
    #[must_use]
    pub fn summary(self) -> String {
        match self {
            Self::Shutdown => tr!("0x40 WillShutdown (id only, what macOS sends)"),
            Self::DeviceName => tr!(
                "0x55 LongDeviceName (64 bytes, Lion; akmctl rename --device-name: \
pre-flight, backup, one confirmation, read back)"
            ),
            Self::Forget => tr!("0x41 RecantConnection (id only, akmctl repair)"),
        }
    }

    /// The only Feature ids this operation may write.
    #[must_use]
    pub const fn ids(self) -> &'static [u8] {
        match self {
            Self::Shutdown => &[0x40],
            Self::DeviceName => &[0x55],
            Self::Forget => &[0x41],
        }
    }

    /// Exact number of data bytes after the id: 0 = the id alone.
    #[must_use]
    pub const fn payload_len(self) -> usize {
        match self {
            Self::Shutdown | Self::Forget => 0,
            Self::DeviceName => 64,
        }
    }

    /// The operation that owns this Feature id, if any.
    #[must_use]
    pub fn of_id(id: u8) -> Option<WriteOp> {
        Self::ALL.into_iter().find(|op| op.ids().contains(&id))
    }
}

#[cfg(test)]
/// Every Feature id some named operation may write (sorted, no duplicate).
#[must_use]
pub fn writable_feature_ids() -> Vec<u8> {
    let mut v: Vec<u8> = WriteOp::ALL
        .iter()
        .flat_map(|op| op.ids().iter().copied())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// May this report be written by *some* named Apple operation?
///
/// # Errors
///
/// [`Refusal`] when no operation may write it.
pub fn check_write(id: u8, dir: Direction) -> Result<(), Refusal> {
    let class = match dir {
        Direction::Feature => classify_feature(id),
        _ => lookup(id, dir).map_or(Safety::Unknown, |e| e.safety),
    };
    if dir == Direction::Feature && class == Safety::WriteApple && WriteOp::of_id(id).is_some() {
        return Ok(());
    }
    Err(Refusal {
        id,
        class,
        write: true,
    })
}

/// May operation `op` write this report?
///
/// # Errors
///
/// [`WriteRefusal`] when `op` may not write it.
pub fn check_write_op(op: WriteOp, id: u8, dir: Direction) -> Result<(), WriteRefusal> {
    check_write(id, dir).map_err(WriteRefusal::Map)?;
    if !op.ids().contains(&id) {
        return Err(WriteRefusal::WrongOperation { op, id });
    }
    Ok(())
}

/// Why [`WriteSession::authorize`] refused a write that the register map allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRefusal {
    /// The register map refuses this id or direction.
    Map(Refusal),
    /// The id belongs to another operation.
    WrongOperation { op: WriteOp, id: u8 },
    /// The data length is not the operation's exact length.
    Payload { id: u8, len: usize, expected: usize },
    /// This id was already written in this session.
    Repeated { id: u8 },
}

impl fmt::Display for WriteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Map(r) => r.fmt(f),
            Self::WrongOperation { op, id } => write!(
                f,
                "write of report {id:#04x} refused: not an id of the operation {}",
                op.as_str()
            ),
            Self::Payload { id, len, expected } => write!(
                f,
                "write of report {id:#04x} refused: {len} data byte(s), Apple sends exactly {expected}"
            ),
            Self::Repeated { id } => {
                write!(f, "write of report {id:#04x} refused: already sent in this session")
            }
        }
    }
}

impl std::error::Error for WriteRefusal {}

impl From<WriteRefusal> for std::io::Error {
    fn from(r: WriteRefusal) -> Self {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, r)
    }
}

/// The "once per session" rule of every write: each id is authorised at most once.
#[derive(Debug)]
pub struct WriteSession {
    used: [bool; 256],
}

impl Default for WriteSession {
    fn default() -> Self {
        Self::new()
    }
}

impl WriteSession {
    #[must_use]
    pub const fn new() -> Self {
        Self { used: [false; 256] }
    }

    #[cfg(any(test, feature = "testseam"))]
    /// Has any write been authorised already?
    #[must_use]
    pub fn is_used(&self) -> bool {
        self.used.iter().any(|&u| u)
    }

    /// Has one of `op`'s ids been authorised already?
    #[must_use]
    pub fn is_used_by(&self, op: WriteOp) -> bool {
        op.ids().iter().any(|&id| self.used[usize::from(id)])
    }

    /// Authorise (and consume) the write of Feature `id` by `op` with `payload` (the bytes after
    /// the id): register map, then the operation's own ids, then its exact length, then once.
    ///
    /// # Errors
    ///
    /// [`WriteRefusal`] naming the first check that fails.
    pub fn authorize(
        &mut self,
        op: WriteOp,
        id: u8,
        dir: Direction,
        payload: &[u8],
    ) -> Result<(), WriteRefusal> {
        check_write_op(op, id, dir)?;
        if payload.len() != op.payload_len() {
            return Err(WriteRefusal::Payload {
                id,
                len: payload.len(),
                expected: op.payload_len(),
            });
        }
        if self.used[usize::from(id)] {
            return Err(WriteRefusal::Repeated { id });
        }
        self.used[usize::from(id)] = true;
        Ok(())
    }
}

/// u16 little-endian of the first two payload bytes.
#[must_use]
pub fn u16_le(p: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*p.first()?, *p.get(1)?]))
}

/// u16 big-endian of the first two payload bytes.
#[must_use]
pub fn u16_be(p: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes([*p.first()?, *p.get(1)?]))
}

/// The four battery thresholds of `0x60` (also `0x5A` / `0xEB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thresholds {
    pub full_mv: u16,
    pub low_mv: u16,
    pub critical_mv: u16,
    pub empty_mv: u16,
}

impl Thresholds {
    /// Parse the 8-byte payload (4 x u16 BE).
    #[must_use]
    pub fn parse(p: &[u8]) -> Option<Self> {
        let w = |i: usize| u16_be(p.get(i * 2..)?);
        let t = Self {
            full_mv: w(0)?,
            low_mv: w(1)?,
            critical_mv: w(2)?,
            empty_mv: w(3)?,
        };
        (t.empty_mv > 0
            && t.full_mv > t.low_mv
            && t.low_mv > t.critical_mv
            && t.critical_mv > t.empty_mv)
            .then_some(t)
    }

    #[cfg(test)]
    #[must_use]
    pub fn as_array(&self) -> [u16; 4] {
        [self.full_mv, self.low_mv, self.critical_mv, self.empty_mv]
    }

    /// Margins in mV above each threshold (negative = already below).
    #[must_use]
    pub fn margins(&self, mv: u32) -> [i32; 4] {
        let m = |t: u16| {
            i32::try_from(mv)
                .unwrap_or(i32::MAX)
                .saturating_sub(i32::from(t))
        };
        [
            m(self.full_mv),
            m(self.low_mv),
            m(self.critical_mv),
            m(self.empty_mv),
        ]
    }

    /// Level a voltage falls in.
    #[must_use]
    pub fn level(&self, mv: u32) -> ThresholdLevel {
        if mv > u32::from(self.low_mv) {
            ThresholdLevel::Ok
        } else if mv > u32::from(self.critical_mv) {
            ThresholdLevel::Low
        } else if mv > u32::from(self.empty_mv) {
            ThresholdLevel::Critical
        } else {
            ThresholdLevel::Empty
        }
    }
}

/// Where a voltage sits relative to [`Thresholds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdLevel {
    Ok,
    Low,
    Critical,
    Empty,
}

impl ThresholdLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Low => "low",
            Self::Critical => "critical",
            Self::Empty => "empty",
        }
    }
}

/// State of report `0x30`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    Normal,
    Low,
    Critical,
    /// A value macOS treats as an error.
    Invalid(u8),
}

impl BatteryState {
    /// 0 normal, 1 low, 2 or 3 critical, anything else invalid [disassembly of the macOS driver].
    #[must_use]
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Normal,
            1 => Self::Low,
            2 | 3 => Self::Critical,
            n => Self::Invalid(n),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Low => "low",
            Self::Critical => "critical",
            Self::Invalid(_) => "invalid",
        }
    }
}

/// Percentage "as macOS shows it" for a raw `0x47` value: capped at 100; 54..=100 -> 100.
#[must_use]
pub fn apple_display_percent(raw: u8) -> f64 {
    let apple = &crate::apple_model::APPLE;
    crate::apple_model::display_percent(raw.min(apple.percent_max))
}

/// Does `IOBluetooth` apply [`apple_display_percent`] to this product id?
#[must_use]
pub fn apple_display_applies(pid: u32) -> bool {
    (0x0239..=0x023B).contains(&pid) || (0x0255..=0x0257).contains(&pid)
}

/// A decoded value, ready to display.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    Number(u32),
    Text(String),
    Thresholds(Thresholds),
    State(BatteryState),
    Hex(String),
    /// Understood but secret: never shown.
    Hidden,
}

impl fmt::Display for Decoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Text(s) => write!(f, "\"{s}\""),
            Self::Thresholds(t) => write!(
                f,
                "Full {} / Low {} / Critical {} / Empty {} mV",
                t.full_mv, t.low_mv, t.critical_mv, t.empty_mv
            ),
            Self::State(s) => write!(f, "{}", s.as_str()),
            Self::Hex(h) => write!(f, "{h}"),
            Self::Hidden => write!(f, "(secret, hidden)"),
        }
    }
}

impl Entry {
    /// Decode a payload (bytes after the report id).
    pub fn decode_payload(&self, p: &[u8]) -> Option<Decoded> {
        match self.decoder {
            D::Raw => (!p.is_empty()).then(|| Decoded::Hex(hex(p))),
            D::U8 => p.first().map(|&b| Decoded::Number(u32::from(b))),
            D::Percent => p
                .first()
                .filter(|&&b| b <= 100)
                .map(|&b| Decoded::Number(u32::from(b))),
            D::MilliVoltsLe => u16_le(p).map(|v| Decoded::Number(u32::from(v))),
            D::MilliVoltsBe => u16_be(p).map(|v| Decoded::Number(u32::from(v))),
            D::VersionLe => u16_le(p).map(|v| Decoded::Hex(format!("0x{v:04X}"))),
            D::U16Be => u16_be(p).map(|v| Decoded::Number(u32::from(v))),
            D::Thresholds => Thresholds::parse(p).map(Decoded::Thresholds),
            D::Ascii => {
                let end = p.iter().position(|&b| b == 0).unwrap_or(p.len());
                let s: String = p[..end]
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7f).contains(&b) {
                            char::from(b)
                        } else {
                            '?'
                        }
                    })
                    .collect();
                Some(Decoded::Text(s))
            }
            D::BatteryState => p
                .first()
                .map(|&b| Decoded::State(BatteryState::from_byte(b))),
            D::HostAddress => (p.len() >= 7).then_some(Decoded::Hidden),
        }
    }

    /// Unit-aware text of a decoded value (`2986 mV`, `99 %`).
    #[must_use]
    pub fn render(&self, d: &Decoded) -> String {
        match (d, self.unit) {
            (Decoded::Number(n), "mV" | "%") => format!("{n} {}", self.unit),
            (d, _) => d.to_string(),
        }
    }
}

/// Sorted by direction then id, for display.
#[must_use]
pub fn sorted() -> Vec<&'static Entry> {
    let mut v: Vec<&Entry> = TABLE.iter().collect();
    v.sort_by_key(|e| (e.dir as u8, e.id));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_per_direction() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert!(!(a.id == b.id && a.dir == b.dir), "duplicate {:#04x}", a.id);
            }
        }
    }

    #[test]
    fn every_entry_is_documented() {
        for e in TABLE {
            assert!(!e.name.is_empty() && !e.meaning.is_empty() && !e.source.is_empty());
            // A decoded entry that the daemon reads must have a length.
            if e.safety.daemon_may_read() {
                assert!(e.len.is_some(), "{:#04x}", e.id);
                assert_eq!(e.dir, Direction::Feature);
            }
            if e.safety.daemon_may_read_input() {
                assert!(e.len.is_some(), "{:#04x}", e.id);
                assert_eq!(e.dir, Direction::Input);
            }
        }
    }

    #[test]
    fn generated_lists_match_the_policy_of_the_issue() {
        assert_eq!(SAFE_READ_IDS, [0x47, 0x46, 0x49]);
        let mut once = ONCE_PER_CONNECTION_IDS.to_vec();
        once.sort_unstable();
        assert_eq!(once, [0x4F, 0x51, 0x52, 0x53, 0x54, 0x60]);
        for id in DAEMON_ONCE_IDS.into_iter().chain(DAEMON_DEFERRED_ONCE_IDS) {
            assert!(ONCE_PER_CONNECTION_IDS.contains(&id));
        }
    }

    #[test]
    fn named_classes_of_the_specification() {
        let class = |id| classify_feature(id);
        for id in [0x47, 0x46, 0x49] {
            assert_eq!(class(id), Safety::SafeRead);
        }
        for id in [0x4F, 0x60, 0x51, 0x52, 0x53, 0x54] {
            assert_eq!(class(id), Safety::OncePerConnection);
        }
        for id in [0xFE, 0x4C] {
            assert_eq!(class(id), Safety::NeverRead);
        }
        for id in [0x44, 0x45, 0x50, 0xD0, 0xD4, 0xD5, 0xFA, 0xFB] {
            assert_eq!(class(id), Safety::NeverWrite, "{id:#04x}");
        }
        assert_eq!(class(0x40), Safety::WriteApple);
        assert_eq!(class(0x55), Safety::WriteApple);
        assert_eq!(class(0x41), Safety::WriteApple);
        for id in [0x04, 0x05, 0x13, 0x11, 0x12] {
            assert_eq!(
                lookup(id, Direction::Input).unwrap().safety,
                Safety::PassiveInput
            );
        }
        assert_eq!(
            lookup(0x30, Direction::Input).unwrap().safety,
            Safety::SafeReadInput
        );
        assert_eq!(
            lookup(0x01, Direction::Input).unwrap().safety,
            Safety::NeverRead
        );
    }

    #[test]
    fn input_read_gate_allows_exactly_0x30() {
        let mut ok = Vec::new();
        for id in 0..=255u8 {
            match check_read_input(id) {
                Ok(c) => {
                    assert_eq!(c, Safety::SafeReadInput);
                    ok.push(id);
                }
                Err(r) => {
                    assert!(!r.write);
                    assert_eq!(r.id, id);
                    let e: std::io::Error = r.into();
                    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
                }
            }
            assert_eq!(
                check_read_input(id).is_ok(),
                classify_input(id).daemon_may_read_input(),
                "{id:#04x}"
            );
        }
        assert_eq!(ok, vec![0x30]);
        assert_eq!(SAFE_READ_INPUT_IDS, [0x30]);
        // The Feature gate never lets 0x30 through, the Input gate never a Feature id.
        assert!(check_read(0x30).is_err());
        for id in SAFE_READ_IDS.into_iter().chain(ONCE_PER_CONNECTION_IDS) {
            assert!(check_read_input(id).is_err(), "{id:#04x}");
        }
        for s in [
            Safety::SafeRead,
            Safety::OncePerConnection,
            Safety::PassiveInput,
            Safety::ManualOnly,
            Safety::NeverRead,
            Safety::WriteApple,
            Safety::NeverWrite,
            Safety::Unknown,
        ] {
            assert!(!s.daemon_may_read_input(), "{s:?}");
        }
        assert!(
            !Safety::SafeReadInput.daemon_may_read(),
            "an Input class is no Feature permission"
        );
        assert_eq!(Safety::SafeReadInput.as_str(), "SafeReadInput");
    }

    #[test]
    fn unlisted_ids_are_never_readable() {
        for id in 0..=255u8 {
            if lookup_feature(id).is_none() {
                let c = classify_feature(id);
                assert!(!c.daemon_may_read());
                assert_eq!(c == Safety::NeverWrite, id >> 4 == 0xD || id >> 4 == 0xF);
            }
        }
        assert_eq!(classify_feature(0xD9), Safety::NeverWrite);
        assert_eq!(classify_feature(0x77), Safety::Unknown);
    }

    #[test]
    fn read_gate_matches_the_class_for_every_id() {
        for id in 0..=255u8 {
            let ok = check_read(id).is_ok();
            assert_eq!(ok, classify_feature(id).daemon_may_read(), "{id:#04x}");
            if !ok {
                let e: std::io::Error = check_read(id).unwrap_err().into();
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
            }
        }
    }

    #[test]
    fn sco_audio_hint_0x4a_stays_refused() {
        assert!(NEVER.contains(&0x4A));
        assert_eq!(WriteOp::of_id(0x4A), None);
        assert!(!writable_feature_ids().contains(&0x4A));
        assert!(check_write(0x4A, Direction::Feature).is_err());
        let mut s = WriteSession::new();
        for op in WriteOp::ALL {
            assert!(
                s.authorize(op, 0x4A, Direction::Feature, &[0x03]).is_err(),
                "{op:?}"
            );
        }
        assert!(!s.is_used(), "a refusal consumes nothing");
    }

    const NEVER: [u8; 18] = [
        0x44, 0x45, 0x4A, 0x4C, 0x09, 0xD5, 0x50, 0x51, 0x52, 0x53, 0x54, 0xD0, 0xD4, 0xFB, 0xFA,
        0x43, 0xC6, 0xDC,
    ];

    #[test]
    fn only_the_ids_of_the_named_operations_are_writable() {
        let mut ok = Vec::new();
        for id in 0..=255u8 {
            for dir in [Direction::Feature, Direction::Input, Direction::Output] {
                match check_write(id, dir) {
                    Ok(()) => ok.push((id, dir)),
                    Err(r) => {
                        assert!(r.write);
                        assert!(r.to_string().contains("refused"));
                        let e: std::io::Error = r.into();
                        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
                    }
                }
            }
        }
        let expected: Vec<(u8, Direction)> = writable_feature_ids()
            .into_iter()
            .map(|id| (id, Direction::Feature))
            .collect();
        assert_eq!(ok, expected);
        assert_eq!(writable_feature_ids(), vec![0x40, 0x41, 0x55]);
        assert_eq!(WriteOp::of_id(0x41), Some(WriteOp::Forget));
        assert_eq!(WriteOp::of_id(0x40), Some(WriteOp::Shutdown));
        assert_eq!(WriteOp::of_id(0x55), Some(WriteOp::DeviceName));
        for id in NEVER.into_iter().chain(0xD0..=0xFB) {
            assert!(check_write(id, Direction::Feature).is_err(), "{id:#04x}");
            for op in WriteOp::ALL {
                assert!(
                    check_write_op(op, id, Direction::Feature).is_err(),
                    "{id:#04x}"
                );
            }
        }
        for id in writable_feature_ids() {
            assert!(check_write(id, Direction::Input).is_err());
            assert!(check_write(id, Direction::Output).is_err());
            assert!(!classify_feature(id).daemon_may_read());
            assert!(check_read(id).is_err());
        }
        let mut class_ids: Vec<u8> = TABLE
            .iter()
            .filter(|e| e.safety == Safety::WriteApple)
            .map(|e| {
                assert_eq!(e.dir, Direction::Feature);
                e.id
            })
            .collect();
        class_ids.sort_unstable();
        assert_eq!(class_ids, writable_feature_ids());
    }

    #[test]
    fn each_id_belongs_to_one_operation_only() {
        for op in WriteOp::ALL {
            for id in 0..=255u8 {
                let r = check_write_op(op, id, Direction::Feature);
                assert_eq!(
                    r.is_ok(),
                    op.ids().contains(&id),
                    "{} {id:#04x}",
                    op.as_str()
                );
                if op.ids().contains(&id) {
                    assert_eq!(WriteOp::of_id(id), Some(op));
                }
            }
        }
        assert_eq!(
            check_write_op(WriteOp::Shutdown, 0x55, Direction::Feature),
            Err(WriteRefusal::WrongOperation {
                op: WriteOp::Shutdown,
                id: 0x55
            })
        );
        assert!(
            check_write_op(WriteOp::DeviceName, 0x40, Direction::Feature)
                .unwrap_err()
                .to_string()
                .contains("DeviceName")
        );
    }

    #[test]
    fn session_authorises_each_id_once_with_its_exact_length() {
        // Over the 256 ids and every operation.
        for op in WriteOp::ALL {
            let data = vec![0u8; op.payload_len()];
            for id in 0..=255u8 {
                let mut s = WriteSession::new();
                let r = s.authorize(op, id, Direction::Feature, &data);
                assert_eq!(
                    r.is_ok(),
                    op.ids().contains(&id),
                    "{} {id:#04x}",
                    op.as_str()
                );
                assert_eq!(
                    s.is_used(),
                    r.is_ok(),
                    "a refused id must not consume the session"
                );
                for dir in [Direction::Input, Direction::Output] {
                    assert!(WriteSession::new().authorize(op, id, dir, &data).is_err());
                }
            }
        }
        let mut s = WriteSession::new();
        assert!(s
            .authorize(WriteOp::Shutdown, 0x40, Direction::Feature, &[])
            .is_ok());
        assert!(s.is_used_by(WriteOp::Shutdown) && !s.is_used_by(WriteOp::DeviceName));
        let again = s
            .authorize(WriteOp::Shutdown, 0x40, Direction::Feature, &[])
            .unwrap_err();
        assert_eq!(again, WriteRefusal::Repeated { id: 0x40 });
        assert!(again.to_string().contains("already sent"));
        assert!(matches!(
            s.authorize(WriteOp::Shutdown, 0x44, Direction::Feature, &[]),
            Err(WriteRefusal::Map(_))
        ));
        let mut s = WriteSession::new();
        for bad in [0usize, 1, 8, 32, 63, 65, 255] {
            assert_eq!(
                s.authorize(WriteOp::DeviceName, 0x55, Direction::Feature, &vec![0; bad])
                    .unwrap_err(),
                WriteRefusal::Payload {
                    id: 0x55,
                    len: bad,
                    expected: 64
                }
            );
        }
        assert!(!s.is_used());
        assert!(s
            .authorize(WriteOp::DeviceName, 0x55, Direction::Feature, &[0x41; 64])
            .is_ok());
        assert_eq!(
            s.authorize(WriteOp::DeviceName, 0x55, Direction::Feature, &[0x41; 64])
                .unwrap_err(),
            WriteRefusal::Repeated { id: 0x55 }
        );
        assert!(s
            .authorize(WriteOp::Shutdown, 0x40, Direction::Feature, &[])
            .is_ok());
        let mut s = WriteSession::new();
        assert_eq!(
            s.authorize(WriteOp::Shutdown, 0x40, Direction::Feature, &[3])
                .unwrap_err(),
            WriteRefusal::Payload {
                id: 0x40,
                len: 1,
                expected: 0
            }
        );
        assert!(!s.is_used());
        let e: std::io::Error = WriteRefusal::Repeated { id: 0x40 }.into();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn measured_frames_decode() {
        let get = |id| lookup_feature(id).unwrap();
        assert_eq!(
            get(0x46).decode_payload(&[0xaa, 0x0b]),
            Some(Decoded::Number(2986))
        );
        assert_eq!(
            get(0x49).decode_payload(&[0x86, 0x0b]),
            Some(Decoded::Number(2950))
        );
        assert_eq!(
            get(0xFF).decode_payload(&[0x0b, 0xaa, 0x01]),
            Some(Decoded::Number(2986))
        );
        assert_eq!(get(0x47).decode_payload(&[0x63]), Some(Decoded::Number(99)));
        assert_eq!(
            get(0x47).decode_payload(&[0x65]),
            None,
            "above 100 is corrupt"
        );
        assert_eq!(
            get(0x4F).decode_payload(&[0x50, 0x00]),
            Some(Decoded::Hex("0x0050".into()))
        );
        assert_eq!(
            get(0xF5).decode_payload(&[0x03, 0x84]),
            Some(Decoded::Number(900))
        );
        assert_eq!(
            get(0x51).decode_payload(b"Alice's "),
            Some(Decoded::Text("Alice's ".into()))
        );
        assert_eq!(
            get(0x53).decode_payload(b" #1\0\0\0\0\0"),
            Some(Decoded::Text(" #1".into()))
        );
        assert_eq!(
            get(0x4C).decode_payload(&[3, 1, 2, 3, 4, 5, 6, 9]),
            Some(Decoded::Hidden)
        );
        assert_eq!(get(0x4C).decode_payload(&[3]), None);
    }

    #[test]
    fn thresholds_of_the_unit() {
        let p = [0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06];
        let t = Thresholds::parse(&p).unwrap();
        assert_eq!(t.as_array(), [2954, 2506, 2404, 2054]);
        assert_eq!(t.level(2950), ThresholdLevel::Ok);
        assert_eq!(t.level(2506), ThresholdLevel::Low);
        assert_eq!(t.level(2404), ThresholdLevel::Critical);
        assert_eq!(t.level(2054), ThresholdLevel::Empty);
        assert_eq!(t.margins(2986), [32, 480, 582, 932]);
        assert_eq!(t.margins(2000)[3], -54);
        assert_eq!(Thresholds::parse(&p[..7]), None);
        assert_eq!(Thresholds::parse(&[0; 8]), None);
        assert_eq!(
            Thresholds::parse(&[0x0b, 0x8a, 0x0b, 0x8a, 0x09, 0x64, 0x08, 0x06]),
            None
        );
    }

    #[test]
    fn battery_state_values() {
        assert_eq!(BatteryState::from_byte(0), BatteryState::Normal);
        assert_eq!(BatteryState::from_byte(1), BatteryState::Low);
        assert_eq!(BatteryState::from_byte(2), BatteryState::Critical);
        assert_eq!(BatteryState::from_byte(3), BatteryState::Critical);
        assert_eq!(BatteryState::from_byte(4), BatteryState::Invalid(4));
    }

    #[allow(clippy::float_cmp)] // reason: exact values are the property under test
    #[test]
    fn apple_curve_of_the_issue() {
        assert_eq!(apple_display_percent(100), 100.0);
        assert_eq!(apple_display_percent(54), 100.0);
        assert_eq!(apple_display_percent(200), 100.0, "capped at 100 first");
        assert_eq!(apple_display_percent(53).round(), 99.0);
        assert!((apple_display_percent(40) - 67.3125).abs() < 1e-9);
        assert!((apple_display_percent(30) - 42.9).abs() < 0.05);
        assert_eq!(apple_display_percent(21), 21.0);
        assert_eq!(apple_display_percent(20), 20.0);
        assert_eq!(apple_display_percent(0), 0.0);
    }

    #[test]
    fn apple_curve_is_monotonic_bounded_and_applies_to_the_right_pids() {
        let mut prev = -1.0;
        for raw in 0..=255u8 {
            let p = apple_display_percent(raw);
            assert!((0.0..=100.0).contains(&p));
            if raw <= 100 {
                assert!(p >= prev - 1e-9 || raw == 0, "monotonic at {raw}");
                prev = p;
            }
        }
        for pid in [0x239, 0x23A, 0x23B, 0x255, 0x256, 0x257] {
            assert!(apple_display_applies(pid));
        }
        for pid in [0x238, 0x23C, 0x254, 0x258, 0x22C, 0x29C, 0] {
            assert!(!apple_display_applies(pid));
        }
    }

    #[test]
    fn decoders_never_panic_and_stay_bounded() {
        for e in TABLE {
            for len in 0..=12usize {
                for fill in [0x00u8, 0x01, 0x7f, 0x80, 0xff] {
                    let p = vec![fill; len];
                    if let Some(d) = e.decode_payload(&p) {
                        let r = e.render(&d);
                        assert!(
                            r.len() <= 3 * len + 16,
                            "{:#04x} {len} {fill:#x}: {r}",
                            e.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hostaddress_never_leaks_its_bytes() {
        let e = lookup_feature(0x4C).unwrap();
        let d = e
            .decode_payload(&[3, 0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0xaa, 0xbb])
            .unwrap();
        let shown = e.render(&d);
        assert_eq!(shown, "(secret, hidden)");
    }

    #[test]
    fn this_build_has_one_write_path() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        for d in [
            "akm-core/src",
            "apple-kb-monitord/src",
            "crates/akmctl/src",
            "crates/akm-helper/src",
        ] {
            let dir = root.join(d);
            assert!(dir.is_dir(), "{} missing", dir.display());
            collect_rs(&dir, &mut files);
        }
        assert!(files.len() > 20);
        let banned = ["HIDIOCSOUTPUT", "HIDIOCSINPUT", "SET_REPORT", "0xC1004806"];
        let mut writers = 0;
        for f in files {
            if f.ends_with("registry.rs") {
                continue; // this test names the tokens it forbids
            }
            let text = std::fs::read_to_string(&f).unwrap();
            let code = |l: &&str| !l.trim_start().starts_with("//");
            for b in banned {
                assert!(
                    !text.lines().filter(code).any(|l| l.contains(b)),
                    "{} mentions {b} outside a comment",
                    f.display()
                );
            }
            let prod = text.split("#[cfg(test)]").next().unwrap();
            let uses = prod
                .lines()
                .filter(code)
                .filter(|l| l.contains("HIDIOCSFEATURE"))
                .count();
            if uses > 0 {
                assert!(
                    f.ends_with("akm-core/src/hidraw.rs"),
                    "{} names HIDIOCSFEATURE",
                    f.display()
                );
                assert_eq!(uses, 4, "{}", f.display());
                assert_eq!(
                    prod.lines()
                        .filter(code)
                        .filter(|l| l.contains("HIDIOCSFEATURE_1"))
                        .count(),
                    2
                );
                assert_eq!(
                    prod.lines()
                        .filter(code)
                        .filter(|l| l.contains("HIDIOCSFEATURE_65"))
                        .count(),
                    2
                );
                writers += 1;
            }
        }
        assert_eq!(writers, 1);
    }

    fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_rs(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
}
