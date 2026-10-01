//! Register map of the Apple A1314 (BCM2042) HID reports, as a declarative
//! table (#219 and docs/RE-*.md, docs/HARDWARE-RAPPORTS-HID.md).
//!
//! Every report id the reverse engineering touched is listed here with its
//! direction, size, Apple name when known, meaning, unit / endianness,
//! decoder, level of proof and a **safety class**. The class is the only
//! authority on what the code may do with an id:
//!
//! | class | rule |
//! |---|---|
//! | [`Safety::SafeRead`] | may be requested in routine (spaced, breaker, one reader) |
//! | [`Safety::OncePerConnection`] | may be requested once per connection |
//! | [`Safety::PassiveInput`] | only listened to on the hidraw node, never requested |
//! | [`Safety::ManualOnly`] | readable and harmless in principle, but never requested by the daemon (RE tools only) |
//! | [`Safety::NeverRead`] | never requested (secret, or freezes the firmware) |
//! | [`Safety::NeverWrite`] | write-only or command registers: no write path exists |
//! | [`Safety::Unknown`] | not understood: neither read nor written |
//!
//! The allow-lists of [`crate::read_policy`] are **generated from this table**
//! ([`SAFE_READ_IDS`], [`ONCE_PER_CONNECTION_IDS`]); the only function that
//! talks to the hardware ([`crate::hidraw::hid_read_feature`]) calls
//! [`check_read`], and no write function exists ([`check_write`] refuses
//! everything). Proof levels: `[mesuré]` observed on the A1314 ISO,
//! `[plist]` Apple driver property list, `[désassemblage]` Apple binaries,
//! `[source]` public document, `[hypothèse]` plausible, not proven.
//!
//! Every decoder here is pure and total: arbitrary bytes never panic.

use std::fmt;

use serde::{Deserialize, Serialize};

// ── vocabulary ─────────────────────────────────────────────────────────────

/// Direction of a HID report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Feature,
    Input,
    Output,
}

impl Direction {
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
    PassiveInput,
    ManualOnly,
    NeverRead,
    NeverWrite,
    Unknown,
}

impl Safety {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SafeRead => "SafeRead",
            Self::OncePerConnection => "OncePerConnection",
            Self::PassiveInput => "PassiveInput",
            Self::ManualOnly => "ManualOnly",
            Self::NeverRead => "NeverRead",
            Self::NeverWrite => "NeverWrite",
            Self::Unknown => "Unknown",
        }
    }

    /// May the daemon request this report on the hardware (GET_REPORT)?
    pub fn daemon_may_read(self) -> bool {
        matches!(self, Self::SafeRead | Self::OncePerConnection)
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
    pub fn tag(self) -> &'static str {
        match self {
            Self::Measured => "[mesuré]",
            Self::Plist => "[plist]",
            Self::Disassembly => "[désassemblage]",
            Self::Source => "[source]",
            Self::Hypothesis => "[hypothèse]",
        }
    }
}

/// Byte order of the multi-byte fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    None,
    Little,
    Big,
    /// Both conventions in one report (0x5B, 0xFF).
    Mixed,
}

impl Endian {
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
    /// Total length returned by GET_REPORT, id included (`None` = n/a).
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

#[allow(clippy::too_many_arguments)]
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

// ── the table ──────────────────────────────────────────────────────────────

/// The register map. Order: safe reads (in the order the daemon requests
/// them), once-per-connection, passive inputs, then the rest by id.
pub const TABLE: &[Entry] = &[
    // ── SafeRead (routine) ────────────────────────────────────────────────
    r(0x47, F, Some(2), Some("BatteryPercent"), "battery_percent",
      "Battery strength computed by the firmware (declared Input, read as Feature by the kernel)",
      "%", E::None, D::Percent, P::Measured, S::SafeRead, "HARDWARE-RAPPORTS-HID §1-2, RE-PILOTE-MACOS §3"),
    r(0x46, F, Some(3), None, "battery_voltage_instant",
      "Instantaneous battery voltage of the cell pair (same quantity as 0xFF)",
      "mV", E::Little, D::MilliVoltsLe, P::Measured, S::SafeRead, "HARDWARE-RAPPORTS-HID §3"),
    r(0x49, F, Some(3), Some("BatteryVoltage"), "battery_voltage_latched",
      "Latched battery voltage (\"MVLT\" of Lion), slow, noiseless",
      "mV", E::Little, D::MilliVoltsLe, P::Plist, S::SafeRead, "RE-PILOTES-ANCIENS §3-4"),
    // ── OncePerConnection ─────────────────────────────────────────────────
    r(0x4F, F, Some(3), None, "firmware_version",
      "Firmware version (= bcdDevice of the modalias)",
      "version", E::Little, D::VersionLe, P::Measured, S::OncePerConnection, "HARDWARE-RAPPORTS-HID §2"),
    r(0x60, F, Some(9), Some("CalibratedBatteryThresholds3"), "battery_thresholds",
      "Battery thresholds Full / Low / Critical / Empty (4 x u16)",
      "mV", E::Big, D::Thresholds, P::Plist, S::OncePerConnection, "RE-PILOTES-ANCIENS §3-4"),
    r(0x51, F, Some(9), Some("DeviceName1"), "device_name_1",
      "Keyboard name, fragment 1 of 4 (8 ASCII bytes)", "text", E::None, D::Ascii, P::Measured,
      S::OncePerConnection, "RE-PILOTE-MACOS §3"),
    r(0x52, F, Some(9), Some("DeviceName2"), "device_name_2",
      "Keyboard name, fragment 2 of 4", "text", E::None, D::Ascii, P::Measured,
      S::OncePerConnection, "RE-PILOTE-MACOS §3"),
    r(0x53, F, Some(9), Some("DeviceName3"), "device_name_3",
      "Keyboard name, fragment 3 of 4", "text", E::None, D::Ascii, P::Measured,
      S::OncePerConnection, "RE-PILOTE-MACOS §3"),
    r(0x54, F, Some(9), Some("DeviceName4"), "device_name_4",
      "Keyboard name, fragment 4 of 4 (empty for short names; name <= 32 bytes)", "text", E::None,
      D::Ascii, P::Measured, S::OncePerConnection, "RE-PILOTE-MACOS §3"),
    // ── PassiveInput ──────────────────────────────────────────────────────
    r(0x30, I, Some(2), Some("BatteryState"), "battery_state",
      "Battery state pushed by the keyboard: 0 normal, 1 low, 2-3 critical",
      "enum", E::None, D::BatteryState, P::Disassembly, S::PassiveInput, "RE-PILOTE-MACOS §3, §6"),
    r(0x04, I, Some(2), None, "sleep",
      "Sleep notification (analogy with the Broadcom reference firmware, unknown to macOS)",
      "-", E::None, D::U8, P::Hypothesis, S::PassiveInput, "RE-COMMANDES-VENDEUR §2.1"),
    r(0x05, I, Some(2), None, "func_lock",
      "Fn-lock state byte (analogy with the Broadcom reference firmware, unknown to macOS)",
      "-", E::None, D::U8, P::Hypothesis, S::PassiveInput, "RE-COMMANDES-VENDEUR §2.1"),
    r(0x13, I, Some(2), None, "wake",
      "Bit 0 device ready, bit 1 connection request (a keyboard that is switched off reports bit 1 = 0)",
      "bits", E::None, D::U8, P::Disassembly, S::PassiveInput, "RE-PILOTES-ANCIENS §7"),
    r(0x11, I, Some(2), None, "eject_fn",
      "Consumer Eject and vendor Fn key", "bits", E::None, D::U8, P::Measured, S::PassiveInput,
      "HARDWARE-RAPPORTS-HID §1"),
    r(0x12, I, Some(2), None, "media_keys",
      "Play/Pause, Fast Forward, Rewind, Next, Previous", "bits", E::None, D::U8, P::Measured,
      S::PassiveInput, "HARDWARE-RAPPORTS-HID §1"),
    // ── NeverRead ─────────────────────────────────────────────────────────
    r(0x4C, F, Some(20), None, "pairing_record",
      "Byte 1 = 0x03, bytes 2-7 = paired host address (reversed), then 12 secret bytes (probable link-key material)",
      "-", E::Mixed, D::HostAddress, P::Measured, S::NeverRead, "HARDWARE-RAPPORTS-HID §2bis"),
    r(0xFE, F, Some(9), None, "freezes_firmware",
      "Answer box; reading it froze the firmware twice (#175)", "-", E::None, D::Raw, P::Measured,
      S::NeverRead, "RE-COMMANDES-VENDEUR §5"),
    r(0x01, I, Some(9), None, "boot_keyboard",
      "Boot keyboard report: key codes (never read nor logged, passive keylogging risk)", "-",
      E::None, D::Raw, P::Source, S::NeverRead, "HARDWARE-RAPPORTS-HID §1"),
    r(0x34, F, None, None, "magic_kb_address",
      "Magic Keyboard only (CVE-2024-0230): address and name; refused by the A1314", "-", E::None,
      D::Raw, P::Source, S::NeverRead, "RE-COMMANDES-VENDEUR §3.3"),
    r(0x35, F, None, None, "magic_kb_link_key",
      "Magic Keyboard only (CVE-2024-0230): link key; refused by the A1314", "-", E::None,
      D::Raw, P::Source, S::NeverRead, "RE-COMMANDES-VENDEUR §3.3"),
    // ── NeverWrite: command / write-only registers ────────────────────────
    r(0x01, O, Some(2), None, "led_output",
      "Keyboard LEDs (Caps Lock...): handled by the kernel through evdev, never by us", "bits",
      E::None, D::Raw, P::Disassembly, S::NeverWrite, "RE-PILOTES-ANCIENS §7"),
    r(0x40, F, None, Some("WillShutdown"), "will_shutdown",
      "Command: the host is going to shut down (write-only; sent by macOS at each shutdown)", "-",
      E::None, D::Raw, P::Plist, S::NeverWrite, "RE-PILOTE-MACOS §3, #191"),
    r(0x41, F, None, Some("RecantConnection"), "recant_connection",
      "Command: give up the connection (Apple's virtual cable unplug); high risk", "-", E::None,
      D::Raw, P::Disassembly, S::NeverWrite, "RE-PILOTES-ANCIENS §7"),
    r(0x43, F, Some(2), Some("UserMode"), "user_mode",
      "Declared by Apple (1-3); absent from the 0x0050 firmware (ERR_INVALID_REPORT_ID)", "enum",
      E::None, D::Raw, P::Measured, S::NeverWrite, "RE-PILOTE-MACOS §3"),
    r(0x44, F, None, Some("FullFactoryDefault"), "full_factory_default",
      "Command: forget ALL link keys (Lion's \"remove keyboard\"); re-pairing from every host", "-",
      E::None, D::Raw, P::Disassembly, S::NeverWrite, "RE-PILOTES-ANCIENS §7, #217 (written only with the manager's agreement)"),
    r(0x45, F, None, Some("FactoryDefault"), "factory_default",
      "Command: factory reset, exact effect unknown; no Apple software calls it", "-", E::None,
      D::Raw, P::Disassembly, S::NeverWrite, "RE-PILOTES-ANCIENS §7"),
    r(0x4A, F, Some(2), None, "sco_link_state",
      "Written by Apple (3 / 4 = SCO link active / inactive); read value 0x12 is not in that enumeration",
      "enum", E::None, D::U8, P::Disassembly, S::NeverWrite, "RE-MACOS-SILICON §3.3, #216 (written only with the manager's agreement)"),
    r(0x50, F, None, Some("DeviceNameChange"), "device_name_change",
      "Command: validate a name change (4 fragments)", "-", E::None, D::Raw, P::Disassembly,
      S::NeverWrite, "RE-PILOTES-ANCIENS §7"),
    r(0x55, F, Some(65), Some("LongDeviceName"), "long_device_name",
      "Long name, 64 bytes, write-only on this keyboard", "text", E::None, D::Raw, P::Disassembly,
      S::NeverWrite, "RE-PILOTE-MACOS §3"),
    r(0xD0, F, None, None, "d0", "Unknown write-only register (maintenance range of the updater)", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "RE-COMMANDES-VENDEUR §1.3"),
    r(0xD4, F, None, None, "d4", "Unknown write-only register (maintenance range of the updater)", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "RE-COMMANDES-VENDEUR §1.3"),
    r(0xD5, F, None, None, "d5", "Unknown write-only register (maintenance range of the updater)", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "RE-COMMANDES-VENDEUR §1.3"),
    r(0xD1, F, Some(2), None, "d1", "Unknown, constant 0", "-", E::None, D::U8, P::Measured,
      S::NeverWrite, "HARDWARE-RAPPORTS-HID §2"),
    r(0xD8, F, Some(2), None, "d8", "Unknown, constant 0", "-", E::None, D::U8, P::Measured,
      S::NeverWrite, "HARDWARE-RAPPORTS-HID §2"),
    r(0xFA, F, None, None, "fa", "Unknown write-only register, unknown to every Apple software examined", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "RE-PILOTES-ANCIENS §7"),
    r(0xFB, F, None, None, "fb", "Unknown write-only register, unknown to every Apple software examined", "-",
      E::None, D::Raw, P::Hypothesis, S::NeverWrite, "RE-PILOTES-ANCIENS §7"),
    r(0xF6, F, Some(3), None, "f6", "Unknown, constant 4", "-", E::Big, D::U16Be, P::Measured,
      S::NeverWrite, "HARDWARE-RAPPORTS-HID §2"),
    r(0xF7, F, Some(3), None, "f7", "Unknown, constant 4", "-", E::Big, D::U16Be, P::Measured,
      S::NeverWrite, "HARDWARE-RAPPORTS-HID §2"),
    r(0xC6, F, None, None, "connection_interval_update",
      "Apple: connection interval update (5-byte SET); absent from this keyboard", "-", E::None,
      D::Raw, P::Disassembly, S::NeverWrite, "RE-MACOS-SILICON §3.3"),
    r(0xDC, F, None, None, "llr",
      "Apple: low latency / reliable radio setting (3-byte SET); absent from this keyboard", "-",
      E::None, D::Raw, P::Disassembly, S::NeverWrite, "RE-MACOS-SILICON §3.3"),
    r(0xD7, F, None, Some("SuperMode"), "super_mode",
      "Magic Mouse / Trackpad multitouch switch; refused by the keyboard", "-", E::None, D::Raw,
      P::Source, S::NeverWrite, "RE-COMMANDES-VENDEUR §3.1"),
    r(0xB0, F, None, None, "backlight_set",
      "Magic Keyboard backlight; absent from the A1314 (no backlight)", "-", E::None, D::Raw,
      P::Source, S::NeverWrite, "RE-COMMANDES-VENDEUR §3.4"),
    r(0xBF, F, None, None, "backlight_config",
      "Magic Keyboard backlight configuration; absent from the A1314", "-", E::None, D::Raw,
      P::Source, S::NeverWrite, "RE-COMMANDES-VENDEUR §3.4"),
    r(0x0A, F, None, None, "usb_bootloader",
      "Cypress USB bootloader report of the wired A1243 (Chen 2009); absent from the A1314", "-",
      E::None, D::Raw, P::Source, S::NeverWrite, "RE-COMMANDES-VENDEUR §4"),
    // ── ManualOnly: readable, understood, not worth a daemon request ──────
    r(0xFF, F, Some(4), None, "battery_voltage_be",
      "Same voltage as 0x46 (big-endian) followed by 0x01", "mV", E::Big, D::MilliVoltsBe,
      P::Measured, S::ManualOnly, "HARDWARE-RAPPORTS-HID §3"),
    r(0x5A, F, Some(9), None, "thresholds_copy_5a",
      "Copy of 0x60 (4 x u16)", "mV", E::Big, D::Thresholds, P::Measured, S::ManualOnly,
      "HARDWARE-RAPPORTS-HID §4, RE-PILOTES-ANCIENS §7"),
    r(0xEB, F, Some(9), None, "thresholds_copy_eb",
      "Copy of 0x60 (second set?)", "mV", E::Big, D::Thresholds, P::Measured, S::ManualOnly,
      "RE-PILOTES-ANCIENS §7"),
    r(0x5B, F, Some(9), None, "f4_f5_concat", "0xF4 followed by 0xF5 and four zero bytes", "-",
      E::Big, D::Raw, P::Measured, S::ManualOnly, "HARDWARE-RAPPORTS-HID §2"),
    r(0xF4, F, Some(3), None, "f4", "Constant 1740 (cut-off voltage? radio slots?)", "-", E::Big,
      D::U16Be, P::Hypothesis, S::ManualOnly, "HARDWARE-RAPPORTS-HID §4"),
    r(0xF5, F, Some(3), None, "f5",
      "Constant 900 (idle delay in seconds?). NOT a voltage (#139)", "-", E::Big, D::U16Be,
      P::Hypothesis, S::ManualOnly, "HARDWARE-RAPPORTS-HID §3-4, #173"),
    r(0xEA, F, Some(2), None, "ea",
      "Second percentage estimator? An isolated 0 must be ignored", "%", E::None, D::U8,
      P::Hypothesis, S::ManualOnly, "HARDWARE-RAPPORTS-HID §4-5"),
    r(0x09, F, Some(4), None, "caps_lock_delay_flag",
      "Only declared Feature (FF01:0B): flag of the firmware Caps Lock delay (01 = off)", "flag",
      E::None, D::U8, P::Disassembly, S::ManualOnly, "RE-PILOTES-ANCIENS §7"),
    r(0x5C, F, Some(9), None, "empty_5c", "Always empty", "-", E::None, D::Raw, P::Measured,
      S::ManualOnly, "HARDWARE-RAPPORTS-HID §2"),
    r(0x5D, F, Some(9), None, "empty_5d", "Always empty", "-", E::None, D::Raw, P::Measured,
      S::ManualOnly, "HARDWARE-RAPPORTS-HID §2"),
    // ── Unknown ───────────────────────────────────────────────────────────
    r(0x4B, F, Some(3), None, "unknown_4b", "Unknown, constant 00 08", "-", E::None, D::Raw,
      P::Measured, S::Unknown, "HARDWARE-RAPPORTS-HID §2"),
    r(0x4E, F, Some(11), None, "connection_counts",
      "Apple: connection counters (two u32); absent from this keyboard", "-", E::None, D::Raw,
      P::Disassembly, S::Unknown, "RE-MACOS-SILICON §3.3"),
    r(0x90, I, None, None, "magic_power",
      "Battery of the Magic Keyboard / Mouse 2 (Power page); absent from the A1314", "%", E::None,
      D::Raw, P::Source, S::Unknown, "RE-COMMANDES-VENDEUR §2.3"),
];

// ── generated allow-lists ──────────────────────────────────────────────────

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
const N_ONCE: usize = count(Safety::OncePerConnection, Direction::Feature);

/// Feature ids that may be requested in routine, in table order, generated
/// from [`TABLE`] (the safe read policy iterates this).
pub const SAFE_READ_IDS: [u8; N_SAFE] = ids(Safety::SafeRead, Direction::Feature);
/// Feature ids that may be requested once per connection, generated from [`TABLE`].
pub const ONCE_PER_CONNECTION_IDS: [u8; N_ONCE] =
    ids(Safety::OncePerConnection, Direction::Feature);
/// Of those, the ones the daemon actually requests (once per connection):
/// firmware version and battery thresholds. The name fragments stay
/// available to `akmctl info` but BlueZ already gives the name.
pub const DAEMON_ONCE_IDS: [u8; 2] = [0x4F, 0x60];

// ── lookup and classification ──────────────────────────────────────────────

/// Entry of `(id, dir)`.
pub fn lookup(id: u8, dir: Direction) -> Option<&'static Entry> {
    TABLE.iter().find(|e| e.id == id && e.dir == dir)
}

/// Entry of a Feature report.
pub fn lookup_feature(id: u8) -> Option<&'static Entry> {
    lookup(id, Direction::Feature)
}

/// Safety class of any Feature id: its entry's class, else `NeverWrite` for
/// the unexplored `0xDx` / `0xFx` ranges (maintenance and power registers of
/// the firmware), else `Unknown`.
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

/// May the daemon request this Feature report? The single gate of every read.
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

/// May this report be written? **Never**: no class authorises a write in this
/// version (the two Apple commands that could be, 0x44 and 0x4A, need the
/// manager's explicit agreement, #216 / #217, and are not implemented).
pub fn check_write(id: u8, dir: Direction) -> Result<(), Refusal> {
    let class = match dir {
        Direction::Feature => classify_feature(id),
        _ => lookup(id, dir).map_or(Safety::Unknown, |e| e.safety),
    };
    Err(Refusal {
        id,
        class,
        write: true,
    })
}

// ── decoders ───────────────────────────────────────────────────────────────

/// u16 little-endian of the first two payload bytes.
pub fn u16_le(p: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*p.first()?, *p.get(1)?]))
}

/// u16 big-endian of the first two payload bytes.
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
    /// Parse the 8-byte payload (4 x u16 BE). `None` if short, not strictly
    /// decreasing or containing a zero: a corrupt read is dropped.
    pub fn parse(p: &[u8]) -> Option<Self> {
        let w = |i: usize| u16_be(p.get(i * 2..)?);
        let t = Self {
            full_mv: w(0)?,
            low_mv: w(1)?,
            critical_mv: w(2)?,
            empty_mv: w(3)?,
        };
        (t.empty_mv > 0 && t.full_mv > t.low_mv && t.low_mv > t.critical_mv && t.critical_mv > t.empty_mv)
            .then_some(t)
    }

    pub fn as_array(&self) -> [u16; 4] {
        [self.full_mv, self.low_mv, self.critical_mv, self.empty_mv]
    }

    /// Margins in mV above each threshold (negative = already below).
    pub fn margins(&self, mv: u32) -> [i32; 4] {
        let m = |t: u16| i32::try_from(mv).unwrap_or(i32::MAX).saturating_sub(i32::from(t));
        [m(self.full_mv), m(self.low_mv), m(self.critical_mv), m(self.empty_mv)]
    }

    /// Level a voltage falls in.
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
    /// 0 normal, 1 low, 2 or 3 critical, anything else invalid
    /// [désassemblage RE-PILOTE-MACOS §6].
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Normal,
            1 => Self::Low,
            2 | 3 => Self::Critical,
            n => Self::Invalid(n),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Low => "low",
            Self::Critical => "critical",
            Self::Invalid(_) => "invalid",
        }
    }
}

/// Percentage "as macOS shows it" for a raw `0x47` value (IOBluetooth,
/// PIDs 0x239-0x23B and 0x255-0x257) [désassemblage RE-MACOS-SILICON §3.2,
/// #213]: raw is capped at 100; 54..=100 -> 100; 21..=53 -> 21 + (r-21) x
/// 2.4375; 0..=20 -> r.
pub fn apple_display_percent(raw: u8) -> f64 {
    let r = raw.min(100);
    match r {
        54..=100 => 100.0,
        21..=53 => 21.0 + f64::from(r - 21) * 2.4375,
        _ => f64::from(r),
    }
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

fn hex(p: &[u8]) -> String {
    p.iter().map(|b| format!("{b:02x}")).collect()
}

impl Entry {
    /// Decode a payload (bytes after the report id). `None` if too short or
    /// implausible. Never panics.
    pub fn decode_payload(&self, p: &[u8]) -> Option<Decoded> {
        match self.decoder {
            D::Raw => (!p.is_empty()).then(|| Decoded::Hex(hex(p))),
            D::U8 => p.first().map(|&b| Decoded::Number(u32::from(b))),
            D::Percent => p.first().filter(|&&b| b <= 100).map(|&b| Decoded::Number(u32::from(b))),
            D::MilliVoltsLe => u16_le(p).map(|v| Decoded::Number(u32::from(v))),
            D::MilliVoltsBe => u16_be(p).map(|v| Decoded::Number(u32::from(v))),
            D::VersionLe => u16_le(p).map(|v| Decoded::Hex(format!("0x{v:04X}"))),
            D::U16Be => u16_be(p).map(|v| Decoded::Number(u32::from(v))),
            D::Thresholds => Thresholds::parse(p).map(Decoded::Thresholds),
            D::Ascii => {
                let end = p.iter().position(|&b| b == 0).unwrap_or(p.len());
                let s: String = p[..end]
                    .iter()
                    .map(|&b| if (0x20..0x7f).contains(&b) { char::from(b) } else { '?' })
                    .collect();
                Some(Decoded::Text(s))
            }
            D::BatteryState => p.first().map(|&b| Decoded::State(BatteryState::from_byte(b))),
            D::HostAddress => (p.len() >= 7).then_some(Decoded::Hidden),
        }
    }

    /// Unit-aware text of a decoded value (`2986 mV`, `99 %`).
    pub fn render(&self, d: &Decoded) -> String {
        match (d, self.unit) {
            (Decoded::Number(n), "mV" | "%") => format!("{n} {}", self.unit),
            (d, _) => d.to_string(),
        }
    }
}

/// Entries listed by `akmctl info`: every `(id, dir)` of the table.
pub fn all() -> &'static [Entry] {
    TABLE
}

/// Sorted by direction then id, for display.
pub fn sorted() -> Vec<&'static Entry> {
    let mut v: Vec<&Entry> = TABLE.iter().collect();
    v.sort_by_key(|e| (e.dir as u8, e.id));
    v
}

// ── tests ──────────────────────────────────────────────────────────────────

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
        }
    }

    #[test]
    fn generated_lists_match_the_policy_of_the_issue() {
        assert_eq!(SAFE_READ_IDS, [0x47, 0x46, 0x49]);
        let mut once = ONCE_PER_CONNECTION_IDS.to_vec();
        once.sort_unstable();
        assert_eq!(once, [0x4F, 0x51, 0x52, 0x53, 0x54, 0x60]);
        for id in DAEMON_ONCE_IDS {
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
        for id in [0x44, 0x45, 0x41, 0x40, 0x50, 0x55, 0xD0, 0xD4, 0xD5, 0xFA, 0xFB] {
            assert_eq!(class(id), Safety::NeverWrite, "{id:#04x}");
        }
        for id in [0x04, 0x05, 0x30, 0x13, 0x11, 0x12] {
            assert_eq!(lookup(id, Direction::Input).unwrap().safety, Safety::PassiveInput);
        }
        assert_eq!(lookup(0x01, Direction::Input).unwrap().safety, Safety::NeverRead);
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
    fn nothing_is_ever_writable() {
        for id in 0..=255u8 {
            for dir in [Direction::Feature, Direction::Input, Direction::Output] {
                let r = check_write(id, dir).unwrap_err();
                assert!(r.write);
                assert!(r.to_string().contains("refused"));
            }
        }
    }

    #[test]
    fn measured_frames_decode() {
        let get = |id| lookup_feature(id).unwrap();
        // docs/HARDWARE-RAPPORTS-HID.md §2.
        assert_eq!(get(0x46).decode_payload(&[0xaa, 0x0b]), Some(Decoded::Number(2986)));
        assert_eq!(get(0x49).decode_payload(&[0x86, 0x0b]), Some(Decoded::Number(2950)));
        assert_eq!(get(0xFF).decode_payload(&[0x0b, 0xaa, 0x01]), Some(Decoded::Number(2986)));
        assert_eq!(get(0x47).decode_payload(&[0x63]), Some(Decoded::Number(99)));
        assert_eq!(get(0x47).decode_payload(&[0x65]), None, "above 100 is corrupt");
        assert_eq!(get(0x4F).decode_payload(&[0x50, 0x00]), Some(Decoded::Hex("0x0050".into())));
        assert_eq!(get(0xF5).decode_payload(&[0x03, 0x84]), Some(Decoded::Number(900)));
        assert_eq!(get(0x51).decode_payload(b"Clavier "), Some(Decoded::Text("Clavier ".into())));
        assert_eq!(get(0x53).decode_payload(b" #1\0\0\0\0\0"), Some(Decoded::Text(" #1".into())));
        assert_eq!(get(0x4C).decode_payload(&[3, 1, 2, 3, 4, 5, 6, 9]), Some(Decoded::Hidden));
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
        // not strictly decreasing
        assert_eq!(Thresholds::parse(&[0x0b, 0x8a, 0x0b, 0x8a, 0x09, 0x64, 0x08, 0x06]), None);
    }

    #[test]
    fn battery_state_values() {
        assert_eq!(BatteryState::from_byte(0), BatteryState::Normal);
        assert_eq!(BatteryState::from_byte(1), BatteryState::Low);
        assert_eq!(BatteryState::from_byte(2), BatteryState::Critical);
        assert_eq!(BatteryState::from_byte(3), BatteryState::Critical);
        assert_eq!(BatteryState::from_byte(4), BatteryState::Invalid(4));
    }

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
    fn decoders_never_panic_and_stay_bounded() {
        // Every entry on every payload length 0..=12 of boundary bytes.
        for e in TABLE {
            for len in 0..=12usize {
                for fill in [0x00u8, 0x01, 0x7f, 0x80, 0xff] {
                    let p = vec![fill; len];
                    let _ = e.decode_payload(&p);
                    if let Some(d) = e.decode_payload(&p) {
                        let _ = e.render(&d);
                    }
                }
            }
        }
    }

    #[test]
    fn hostaddress_never_leaks_its_bytes() {
        let e = lookup_feature(0x4C).unwrap();
        let d = e.decode_payload(&[3, 0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0xaa, 0xbb]).unwrap();
        let shown = e.render(&d);
        assert_eq!(shown, "(secret, hidden)");
    }

    #[test]
    fn this_build_has_no_write_path() {
        // No source file of the workspace may issue a SET_REPORT: the only
        // ioctl is HIDIOCGFEATURE.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        for d in ["akm-core/src", "apple-kb-monitord/src", "crates/akmctl/src", "crates/akm-helper/src", "src"] {
            collect_rs(&root.join(d), &mut files);
        }
        assert!(files.len() > 20);
        let banned = ["HIDIOCSFEATURE", "HIDIOCSOUTPUT", "HIDIOCSINPUT", "SET_REPORT", "0xC1004806"];
        for f in files {
            if f.ends_with("registry.rs") {
                continue; // this test names the tokens it forbids
            }
            let text = std::fs::read_to_string(&f).unwrap();
            for b in banned {
                assert!(
                    !text.lines().any(|l| l.contains(b) && !l.trim_start().starts_with("//")),
                    "{} mentions {b} outside a comment",
                    f.display()
                );
            }
        }
    }

    fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
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
