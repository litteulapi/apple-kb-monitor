//! The name stored **in** the keyboard ("nom propre", #248, #192), as
//! opposed to the alias of this computer (`akmctl rename <nom>`, BlueZ
//! `Alias`, [`crate::alias`]), which stays the default way to rename.
//!
//! What Apple does, established by disassembly (E1, docs/RE-NOM-PROPRE-E1.md,
//! reference frames `tests/fixtures/devname/lion_setdevicename_frames.json`):
//! Lion 10.7.5 `-[AppleBluetoothHIDDevice setDeviceName:]` (`0x4d2fe`), for a
//! PID whose personality declares `LongDeviceName` (the 598 = `0x0256` does),
//! refuses an empty name or one of more than 64 UTF-16 units before any frame,
//! converts it to UTF-8 and drops trailing characters until it fits in 64
//! bytes, then sends **one** SET Feature `0x55` of 65 bytes: `0x55`, the UTF-8
//! name, `0x00` padding (`calloc`). Then an HCI Remote Name Request, no HID
//! read, no `0x50`, no `0x51`-`0x54`. Identical in 10.5.8. On the wire:
//! `53 55` + 64 bytes, one frame of 66 bytes if the outgoing MTU of the HIDP
//! control channel is at least 66.
//!
//! What no disassembly can establish (hardware, [`UNKNOWNS`]): the HANDSHAKE of
//! firmware `0x0050` to a SET `0x55` (U5), persistence across a battery change
//! (U3), whether `0x51`-`0x54` reflect `0x55` (U4). Hence [`run`] still refuses
//! the real write unless three locks are all lifted, in this order, before
//! anything is touched:
//!
//! 1. `[apple] allow_device_name_write = true` in `config.toml` (default
//!    **false**, [`crate::config::Config::allow_device_name_write`]);
//! 2. the negotiated outgoing MTU of the L2CAP control channel, read at
//!    pre-flight (read-only `getsockopt(L2CAP_OPTIONS)` on a duplicate of the
//!    socket `bluetoothd` holds, `akm-hid-control inspect`), is known and
//!    ≥ [`MIN_CONTROL_MTU`] (66): Linux `hidp` does not fragment, and an
//!    oversized frame ends the HID session;
//! 3. the interactive confirmation: the exact name typed again.
//!
//! Around the single frame, the guarded envelope is unchanged: pre-flight,
//! backup of `0x51`-`0x54`, one write through the register map and the
//! 65-byte door ([`crate::hidraw::hid_write_feature`]), wait for the
//! reconnection, read back, guided restore. Everything here is pure or goes
//! through [`RenameEnv`]: the tests use a simulated environment and a spy
//! [`FeatureSink`], never the hardware.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::parity::FeatureSink;
use crate::registry::{Direction, Proof, WriteOp, WriteSession};

/// `LongDeviceName` report id [plist] [désassemblage `0x4d444`].
pub const LONG_DEVICE_NAME_ID: u8 = 0x55;
/// Data bytes of `LongDeviceName` [plist] (`size` = 64; `getMaxDeviceNameLength`
/// = 64 [désassemblage `0x4c6e4`]); the report handed to the kernel is 65 bytes.
pub const LONG_DEVICE_NAME_LEN: usize = 64;
/// Longest name Apple's UI and `setDeviceName:` accept, in UTF-16 units
/// (`[nom length] > 64` → `kIOReturnBadArgument`, nothing sent) [désassemblage `0x4d3d3`].
pub const APPLE_MAX_NAME_UTF16: usize = 64;
/// `DeviceName1..4`, read back after a reconnection [plist] [mesuré].
pub const FRAGMENT_IDS: [u8; 4] = [0x51, 0x52, 0x53, 0x54];
/// Data bytes of each fragment [plist] [mesuré].
pub const FRAGMENT_LEN: usize = 8;
/// Longest name accepted here: what `0x51`-`0x54` can show back (4 × 8), so
/// that every write can be verified. Apple's UI allows 64.
pub const MAX_NAME_LEN: usize = FRAGMENT_IDS.len() * FRAGMENT_LEN;
/// Timeout Apple passes to `IOHIDDeviceInterface::setReport` (1000 ms)
/// [désassemblage `0x4d4ba`]: a bound on the HANDSHAKE wait, not a delay between frames.
pub const APPLE_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(1000);
/// Smallest outgoing L2CAP MTU of the HIDP control channel that carries the
/// 65-byte report in one frame: `0x53` + 65 [désassemblage `setReportWL`
/// `0x5ad6`: chunk = MTU − 1]. Linux `hidp` sends the whole report in one
/// `l2cap` message and the kernel refuses a message longer than the channel's
/// `omtu` (`EMSGSIZE`), which `hidp` treats as a fatal transmit error [source
/// `net/bluetooth/hidp/core.c`, `l2cap_chan_send`]. Measured at pre-flight,
/// never assumed.
pub const MIN_CONTROL_MTU: u16 = 66;
/// How long `akmctl` waits for the keyboard to come back after the write.
pub const RECONNECT_WAIT: Duration = Duration::from_secs(180);
/// Minimum battery for a write (else the keyboard must report state `normal`).
pub const MIN_BATTERY_PCT: f64 = 20.0;
/// Subdirectory of `$XDG_STATE_HOME` holding the backups.
pub const STATE_SUBDIR: &str = "apple-kb-monitor";
/// Characters refused although printable ASCII: `\` is the escape character
/// of BlueZ's `info` key file (`Name=`), where `\s`, `\n`... would be read
/// back as other characters. Every other printable ASCII character is legal
/// for BlueZ (UTF-8, ≤ 248 bytes) and for SDP.
pub const FORBIDDEN: &[char] = &['\\'];
/// The configuration lines that lift the first lock.
pub const CONFIG_HOWTO: &str =
    "[apple]\nallow_device_name_write = true   # $XDG_CONFIG_HOME/apple-kb-monitor/config.toml (~/.config/...)";

/// Is the exact byte sequence Apple sends established, byte for byte? Only
/// [`SequenceProof::EstablishedByDisassembly`] lets [`run`] go past the first
/// check; production code passes [`SEQUENCE_PROOF`], the only place that
/// constructs that variant (enforced by a source scan in the tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceProof {
    NotProven,
    /// The construction of the frame (id, encoding, padding, length, order,
    /// absence of any other frame) is read in Apple's own code, with
    /// addresses, on two systems (10.5.8, 10.7.5), and the reference frames
    /// of the fixture match [`frames_for`] byte for byte. It says nothing
    /// about the firmware's reaction: see [`UNKNOWNS`].
    EstablishedByDisassembly,
}

impl SequenceProof {
    pub fn describe(self) -> &'static str {
        match self {
            Self::NotProven => "NotProven: the bytes Apple sends are not established",
            Self::EstablishedByDisassembly => {
                "established by disassembly (E1): IOBluetooth 10.7.5 x86_64 -[AppleBluetoothHIDDevice setDeviceName:] 0x4d2fe, identical in 10.5.8 i386 0x4fc10; reference frames tests/fixtures/devname/lion_setdevicename_frames.json"
            }
        }
    }
}

/// State of the proof: construction established by E1 (docs/RE-NOM-PROPRE-E1.md).
pub const SEQUENCE_PROOF: SequenceProof = SequenceProof::EstablishedByDisassembly;

/// Settled by E1 (docs/RENOMMER-CLAVIER.md §5.3, docs/RE-NOM-PROPRE-E1.md §5).
pub const RESOLVED_BY_DISASSEMBLY: &[&str] = &[
    "U1 content of the 64 bytes: UTF-8 name (trailing characters dropped until <= 64 bytes), 0x00 padding (calloc), no length prefix, no explicit terminator (a 64-byte name has no NUL) [désassemblage 0x4d3fe-0x4d444]",
    "U2 order: ONE SET Feature 0x55 of 65 bytes, then an HCI Remote Name Request; no GET before or after, no 0x50, no 0x51-0x54, no 0x44, no 0x41; empty or > 64 UTF-16 units refused before any frame (kIOReturnBadArgument) [désassemblage 0x4d3d3-0x4d4c8]",
    "U7 0x50 DeviceNameChange validates the 4-fragment path only (PIDs without LongDeviceName); never sent to this keyboard [désassemblage 0x4d525-0x4d6cb]",
    "U6 (Apple side) one frame of 66 bytes when the control channel's outgoing MTU >= 66, else DATC fragments; Linux side: the MTU is READ at pre-flight (akm-hid-control inspect, getsockopt L2CAP_OPTIONS, read-only) and the write is refused below 66 or when unknown",
];

/// What remains unmeasured: hardware facts no disassembly can give. These are
/// RISKS of the real write, told as such to the user.
pub const UNKNOWNS: &[&str] = &[
    "U5 RISK NOT MEASURED: the HANDSHAKE of firmware 0x0050 to a SET 0x55 (the GET is refused 0x03, which says nothing about SET); Apple waits 1000 ms and treats anything but SUCCESSFUL as a failure without retry",
    "U3 RISK NOT MEASURED: persistence across a battery change (the Magic Keyboard descriptor declares its 0x55 Feature 'volatile'); Apple never rewrites the name at reconnection, so the keyboard is expected to store it (deduction, E3)",
    "U4 PARTIAL: Apple does not read 0x51-0x54 back; it asks the HCI remote name at once. Whether the fragments reflect 0x55, and when, is measured here by the read-back after the reconnection (the BlueZ Device1.Name is reported as information only)",
];

/// Passive experiments that would lift the remaining unknowns (no write to the keyboard).
pub const VALIDATION_EXPERIMENTS: &[&str] = &[
    "E1 DONE (U1, U2, U7, U6 Apple side): docs/RE-NOM-PROPRE-E1.md, fixture tests/fixtures/devname/lion_setdevicename_frames.json",
    "E2 (U6 Linux side) is now a pre-flight: `akm-hid-control inspect --mac <MAC>` reads the negotiated L2CAP MTU of the control channel (read-only) and the write is refused below 66",
    "E3 (U3) compare the cached 0x51-0x54, HID_NAME and BlueZ Name before and after a battery change (daemon cache, no new read)",
    "E4 (U3, U5) public HID descriptors of Apple keyboards of the same generation declaring 0x55: Feature flags (volatile / non-volatile) and size",
];

// ── validation ─────────────────────────────────────────────────────────────

/// Why a name is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong(usize),
    /// More than [`APPLE_MAX_NAME_UTF16`] UTF-16 units: what Apple refuses
    /// (`kIOReturnBadArgument`) before building any frame.
    TooLongForApple(usize),
    /// Leading or trailing space: invisible in every UI, refused.
    EdgeSpace,
    Control(u32),
    NotAscii(char),
    Forbidden(char),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty name"),
            Self::TooLong(n) => write!(
                f,
                "{n} characters, at most {MAX_NAME_LEN} (what 0x51-0x54 can show back)"
            ),
            Self::TooLongForApple(n) => write!(
                f,
                "{n} UTF-16 units, Apple refuses above {APPLE_MAX_NAME_UTF16} before any frame (kIOReturnBadArgument)"
            ),
            Self::EdgeSpace => write!(f, "leading or trailing space"),
            Self::Control(c) => write!(f, "control character U+{c:04X}"),
            Self::NotAscii(c) => write!(f, "non-ASCII character {c:?} (the keyboard stores ASCII)"),
            Self::Forbidden(c) => write!(
                f,
                "character {c:?} refused (escape character of BlueZ's key file)"
            ),
        }
    }
}

impl std::error::Error for NameError {}

/// Validate a new name: printable ASCII (`0x20`-`0x7E`), 1 to
/// [`MAX_NAME_LEN`] characters, no space at either end, none of [`FORBIDDEN`].
/// Nothing is trimmed: what is validated is exactly what would be written.
/// Stricter than Apple ([`apple_name_bytes`]): a subset on which the
/// read-back of `0x51`-`0x54` verifies the whole name.
pub fn validate(name: &str) -> Result<String, NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    for c in name.chars() {
        if c.is_control() {
            return Err(NameError::Control(c as u32));
        }
        if !c.is_ascii() {
            return Err(NameError::NotAscii(c));
        }
        if FORBIDDEN.contains(&c) {
            return Err(NameError::Forbidden(c));
        }
    }
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong(name.len()));
    }
    if name.starts_with(' ') || name.ends_with(' ') {
        return Err(NameError::EdgeSpace);
    }
    Ok(name.to_string())
}

/// The name bytes exactly as Apple builds them [désassemblage `0x4d3b9`-`0x4d43f`]:
/// `[nom length]` (UTF-16 units) must be 1..=64, else `kIOReturnBadArgument`
/// before any frame; then `_UTF8StringFromString(nom, 64)`: UTF-8, trailing
/// characters removed one by one while `strlen > 64`, so a character is never
/// cut in the middle. No terminator: the padding comes from the `calloc`.
pub fn apple_name_bytes(name: &str) -> Result<Vec<u8>, NameError> {
    let units = name.encode_utf16().count();
    if units == 0 {
        return Err(NameError::Empty);
    }
    if units > APPLE_MAX_NAME_UTF16 {
        return Err(NameError::TooLongForApple(units));
    }
    let mut s = name.to_string();
    while s.len() > LONG_DEVICE_NAME_LEN {
        s.pop();
    }
    Ok(s.into_bytes())
}

// ── frames ─────────────────────────────────────────────────────────────────

/// One field of a frame and its level of proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Byte range in the report (id = byte 0).
    pub start: usize,
    pub end: usize,
    pub proof: Proof,
    pub what: &'static str,
}

/// One SET Feature report as handed to the kernel (id first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub op: WriteOp,
    pub report: Vec<u8>,
    pub fields: Vec<Field>,
}

impl Frame {
    pub fn id(&self) -> u8 {
        self.report[0]
    }
    pub fn data(&self) -> &[u8] {
        &self.report[1..]
    }
    /// Bytes on the HIDP control channel: `53` (SET_REPORT | Feature),
    /// added by the Bluetooth stack, then the report [désassemblage]
    /// (`IOBluetoothHIDDriver::setReportWL` `0x5ad6`; `hidp` does the same).
    pub fn wire(&self) -> Vec<u8> {
        let mut w = vec![0x53];
        w.extend_from_slice(&self.report);
        w
    }
}

/// The 64 data bytes of `0x55` for the UTF-8 bytes `raw` of a name
/// (≤ 64): `raw`, then `0x00` up to 64 (`calloc(65, 1)` + `strncpy`).
fn long_name_payload(raw: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; LONG_DEVICE_NAME_LEN];
    p[..raw.len()].copy_from_slice(raw);
    p
}

fn long_name_frame(raw: &[u8]) -> Frame {
    let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let mut report = vec![LONG_DEVICE_NAME_ID];
    report.extend_from_slice(&long_name_payload(raw));
    let mut fields = vec![
        Field {
            start: 0,
            end: 1,
            proof: Proof::Disassembly,
            what: "report id 0x55 LongDeviceName, the only report Lion's setDeviceName: writes for a PID that declares it [plist + désassemblage 0x4d444 `movb %bl,(%r13)`]",
        },
        Field {
            start: 1,
            end: 1 + LONG_DEVICE_NAME_LEN,
            proof: Proof::Disassembly,
            what: "64 data bytes (report of 65): size of LongDeviceName in the 598 personality, getMaxDeviceNameLength = 64, setReport length 65 [plist + désassemblage 0x4d4b1]",
        },
    ];
    if n > 0 {
        fields.push(Field {
            start: 1,
            end: 1 + n,
            proof: Proof::Disassembly,
            what: "name as UTF-8 bytes from byte 1, whole characters, no terminator [désassemblage 0x4d416 _UTF8StringFromString, 0x4d43f strncpy(buf+1, utf8, strlen)]",
        });
    }
    if n < LONG_DEVICE_NAME_LEN {
        fields.push(Field {
            start: 1 + n,
            end: 1 + LONG_DEVICE_NAME_LEN,
            proof: Proof::Disassembly,
            what: "0x00 padding to 64 bytes: calloc(65, 1), nothing else written [désassemblage 0x4d407]",
        });
    }
    Frame {
        op: WriteOp::DeviceName,
        report,
        fields,
    }
}

/// The frames of Apple's rename for a validated `name`: exactly one SET
/// Feature `0x55` of 65 bytes (no `0x50`, no `0x51`-`0x54`), the name encoded
/// as Apple does ([`apple_name_bytes`]). Matches the reference frames of
/// `tests/fixtures/devname/lion_setdevicename_frames.json` byte for byte.
pub fn frames_for(name: &str) -> Result<Vec<Frame>, NameError> {
    let name = validate(name)?;
    let raw = apple_name_bytes(&name)?;
    Ok(vec![long_name_frame(&raw)])
}

/// The frames that rewrite a backup: the 32 saved bytes exactly, then NUL.
pub fn frames_for_restore(b: &Backup) -> Result<Vec<Frame>, String> {
    let raw = b.raw()?;
    Ok(vec![long_name_frame(&raw)])
}

/// The 32 bytes `0x51`-`0x54` should read back after `frames` (U4: the first
/// 32 data bytes of `0x55`).
pub fn expected_readback(frames: &[Frame]) -> Vec<u8> {
    frames
        .iter()
        .find(|f| f.id() == LONG_DEVICE_NAME_ID)
        .map(|f| f.data()[..MAX_NAME_LEN].to_vec())
        .unwrap_or_default()
}

/// Hex with spaces.
pub fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex_compact(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Bytes of a hex string (spaces allowed); `None` if malformed.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Dry-run display: every frame, its bytes on the wire, every field with its proof.
pub fn render_frames(frames: &[Frame]) -> String {
    let mut s = String::new();
    for (i, f) in frames.iter().enumerate() {
        s.push_str(&format!(
            "frame {}/{} — operation {}, SET Feature {:#04x}, {} byte(s) handed to the kernel\n  report : {}\n  wire   : {}\n",
            i + 1,
            frames.len(),
            f.op.as_str(),
            f.id(),
            f.report.len(),
            hex(&f.report),
            hex(&f.wire()),
        ));
        for fd in &f.fields {
            s.push_str(&format!(
                "  bytes {:>2}..{:<2} {:<16} {}\n",
                fd.start,
                fd.end,
                fd.proof.tag(),
                fd.what
            ));
        }
    }
    s.push_str(&format!(
        "then: no other frame (no 0x50, no 0x51-0x54, no read) [désassemblage]; HANDSHAKE awaited at most {} ms [désassemblage]; \
         Apple then asks the HCI remote name; here the name is read back (0x51-0x54) after a reconnection\n",
        APPLE_HANDSHAKE_TIMEOUT.as_millis()
    ));
    s
}

// ── reading back ───────────────────────────────────────────────────────────

/// The 32 raw bytes of `0x51`-`0x54` from four frames (id first, as
/// `HIDIOCGFEATURE` returns them). `None` unless all four are complete.
pub fn raw_from_frames(frames: &[Vec<u8>]) -> Option<Vec<u8>> {
    if frames.len() != FRAGMENT_IDS.len() {
        return None;
    }
    let mut raw = Vec::with_capacity(MAX_NAME_LEN);
    for (f, id) in frames.iter().zip(FRAGMENT_IDS) {
        if f.len() < 1 + FRAGMENT_LEN || f[0] != id {
            return None;
        }
        raw.extend_from_slice(&f[1..1 + FRAGMENT_LEN]);
    }
    Some(raw)
}

/// The name in 32 raw bytes: up to the first NUL; `None` if not printable ASCII.
pub fn name_from_raw(raw: &[u8]) -> Option<String> {
    let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let s = &raw[..n];
    s.iter()
        .all(|b| (0x20..=0x7E).contains(b))
        .then(|| String::from_utf8_lossy(s).into_owned())
}

// ── backup ─────────────────────────────────────────────────────────────────

/// What was in `0x51`-`0x54` before any write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    pub schema: u32,
    /// Unix time of the backup.
    pub created_unix: u64,
    pub mac: String,
    /// The four fragments, 8 data bytes each, hex (id not included).
    pub fragments_hex: [String; 4],
    /// The name they spell.
    pub name: String,
    /// Where the bytes come from (`daemon-cache`: read once in this
    /// connection by the daemon, 1 s spacing, no new request).
    pub source: String,
}

impl Backup {
    pub const SCHEMA: u32 = 1;

    pub fn new(mac: &str, raw: &[u8], created_unix: u64, source: &str) -> Result<Self, String> {
        if raw.len() != MAX_NAME_LEN {
            return Err(format!("{} bytes, expected {MAX_NAME_LEN}", raw.len()));
        }
        let name = name_from_raw(raw).ok_or("the saved name is not printable ASCII")?;
        let frag = |i: usize| hex_compact(&raw[i * FRAGMENT_LEN..(i + 1) * FRAGMENT_LEN]);
        Ok(Self {
            schema: Self::SCHEMA,
            created_unix,
            mac: mac.to_string(),
            fragments_hex: [frag(0), frag(1), frag(2), frag(3)],
            name,
            source: source.to_string(),
        })
    }

    /// The 32 saved bytes, checked: 4 × 8 bytes, printable ASCII then NUL
    /// only, and consistent with `name`.
    pub fn raw(&self) -> Result<Vec<u8>, String> {
        if self.schema != Self::SCHEMA {
            return Err(format!("backup schema {} unknown", self.schema));
        }
        let mut raw = Vec::with_capacity(MAX_NAME_LEN);
        for (i, h) in self.fragments_hex.iter().enumerate() {
            let b = unhex(h).ok_or(format!("fragment {} is not hex", i + 1))?;
            if b.len() != FRAGMENT_LEN {
                return Err(format!(
                    "fragment {} has {} bytes, expected {FRAGMENT_LEN}",
                    i + 1,
                    b.len()
                ));
            }
            raw.extend_from_slice(&b);
        }
        let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        if raw[n..].iter().any(|&b| b != 0) {
            return Err(
                "bytes after the NUL terminator: refused (not a name this tool restores)".into(),
            );
        }
        match name_from_raw(&raw) {
            Some(n) if n == self.name && !n.is_empty() => Ok(raw),
            Some(_) => Err("the fragments do not spell the saved name".into()),
            None => Err("the saved bytes are not printable ASCII".into()),
        }
    }
}

/// `$XDG_STATE_HOME/apple-kb-monitor` (fallback `~/.local/state/...`).
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"))
        .join(STATE_SUBDIR)
}

/// `YYYYMMDDTHHMMSSZ` of a Unix time (UTC).
pub fn utc_stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// File name of a backup made at `unix`.
pub fn backup_file_name(prefix: &str, unix: u64) -> String {
    format!("{prefix}-backup-{}.json", utc_stamp(unix))
}

/// Write `json` to a NEW file `dir/name`, mode 0600 (directory 0700), synced
/// to disk. Never overwrites: an existing file is an error.
pub fn write_private_file(dir: &Path, name: &str, json: &str) -> io::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let path = dir.join(name);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    // The umask can only remove bits; set them explicitly anyway.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(json.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    Ok(path)
}

/// Save a backup in `dir`; returns its path.
pub fn write_backup(dir: &Path, b: &Backup) -> io::Result<PathBuf> {
    let json = serde_json::to_string_pretty(b).map_err(io::Error::other)?;
    write_private_file(dir, &backup_file_name("devname", b.created_unix), &json)
}

/// Read and check a backup file.
pub fn read_backup(path: &Path) -> Result<Backup, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let b: Backup = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    b.raw()?;
    Ok(b)
}

// ── pre-flight ─────────────────────────────────────────────────────────────

/// Facts gathered before a write (from the daemon, `akmctl doctor` and the
/// read-only MTU probe).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preflight {
    pub connected: bool,
    pub battery_pct: Option<f64>,
    /// Input `0x30`: `Some(true)` = state `normal`.
    pub battery_state_normal: Option<bool>,
    pub breaker_open: bool,
    /// The last hardware read of the daemon failed or was incomplete.
    pub recent_read_failure: bool,
    /// `akmctl doctor`: connected, link health `connected`, verdict ok/info.
    pub doctor_green: bool,
    /// stdin and stdout are a terminal.
    pub interactive: bool,
    /// Negotiated **outgoing** MTU of the L2CAP control channel (PSM `0x0011`)
    /// to this keyboard, read on the live socket (`getsockopt(L2CAP_OPTIONS)`,
    /// read-only); `None` = unknown = refused (second lock).
    pub control_mtu: Option<u16>,
}

/// One failed pre-flight condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightFail {
    NotConnected,
    Battery,
    BreakerOpen,
    RecentReadFailure,
    DoctorNotGreen,
    NotInteractive,
    /// The MTU of the control channel could not be read.
    ControlMtuUnknown,
    /// The MTU is known and below [`MIN_CONTROL_MTU`].
    ControlMtuTooSmall(u16),
}

impl PreflightFail {
    pub fn describe(&self) -> String {
        match self {
            Self::NotConnected => "keyboard not connected".into(),
            Self::Battery => "battery below 20 % and state not 'normal' (or unknown)".into(),
            Self::BreakerOpen => "circuit breaker open: the keyboard stopped answering".into(),
            Self::RecentReadFailure => "the last hardware read failed or was incomplete".into(),
            Self::DoctorNotGreen => {
                "`akmctl doctor` is not green (link health, pairing, configuration)".into()
            }
            Self::NotInteractive => {
                "not an interactive terminal (stdin and stdout must be a terminal)".into()
            }
            Self::ControlMtuUnknown => format!(
                "outgoing MTU of the L2CAP control channel unknown: it must be read (akm-hid-control inspect, read-only) and be >= {MIN_CONTROL_MTU} before a 66-byte frame is sent"
            ),
            Self::ControlMtuTooSmall(m) => format!(
                "outgoing MTU of the L2CAP control channel is {m}, below {MIN_CONTROL_MTU}: a 66-byte frame cannot be sent in one piece (Linux hidp does not fragment; the HID session would be terminated)"
            ),
        }
    }

    /// Is this the MTU lock (as opposed to the daemon / terminal facts)?
    pub fn is_mtu(&self) -> bool {
        matches!(self, Self::ControlMtuUnknown | Self::ControlMtuTooSmall(_))
    }
}

/// Every failed condition (empty = go).
pub fn preflight(p: &Preflight) -> Vec<PreflightFail> {
    let mut v = Vec::new();
    if !p.connected {
        v.push(PreflightFail::NotConnected);
    }
    let pct_ok = p
        .battery_pct
        .is_some_and(|x| x.is_finite() && x >= MIN_BATTERY_PCT);
    if !(pct_ok || p.battery_state_normal == Some(true)) {
        v.push(PreflightFail::Battery);
    }
    if p.breaker_open {
        v.push(PreflightFail::BreakerOpen);
    }
    if p.recent_read_failure {
        v.push(PreflightFail::RecentReadFailure);
    }
    if !p.doctor_green {
        v.push(PreflightFail::DoctorNotGreen);
    }
    if !p.interactive {
        v.push(PreflightFail::NotInteractive);
    }
    match p.control_mtu {
        None => v.push(PreflightFail::ControlMtuUnknown),
        Some(m) if m < MIN_CONTROL_MTU => v.push(PreflightFail::ControlMtuTooSmall(m)),
        Some(_) => {}
    }
    v
}

// ── orchestration ──────────────────────────────────────────────────────────

/// Outcome of waiting for the keyboard after the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconnect {
    /// A new connection, with the 32 bytes `0x51`-`0x54` read in it and the
    /// name BlueZ shows for the device (`Device1.Name`, informative: Apple's
    /// own check is the HCI remote name; BlueZ may still serve its cache).
    Back {
        raw: Vec<u8>,
        bluez_name: Option<String>,
    },
    /// No new connection with a fresh read before the deadline.
    Timeout,
}

/// Everything [`run`] needs from the outside world (simulated in the tests).
pub trait RenameEnv {
    fn preflight(&mut self) -> Preflight;
    /// The 32 bytes of `0x51`-`0x54` cached by the daemon in this connection.
    fn cached_raw(&mut self) -> Option<Vec<u8>>;
    fn mac(&mut self) -> String;
    fn now_unix(&mut self) -> u64;
    fn save_backup(&mut self, b: &Backup) -> io::Result<PathBuf>;
    /// Show `prompt`, read one line from the terminal; true only if the line
    /// is exactly `expected`.
    fn confirm(&mut self, prompt: &str, expected: &str) -> bool;
    fn sink(&self) -> &dyn FeatureSink;
    /// Tell the user to switch the keyboard off and on, wait at most `max`.
    fn wait_reconnect(&mut self, max: Duration) -> Reconnect;
    /// One line of the decision journal (stderr in `akmctl`).
    fn log(&mut self, line: &str);
}

/// What is being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// A new name (validated again here).
    Rename(String),
    /// The bytes of a backup.
    Restore(Backup),
}

/// End of a [`run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The name is refused.
    InvalidName(String),
    /// The exact sequence is not proven: nothing was touched.
    NotProven,
    /// First lock: `[apple] allow_device_name_write` is not `true`; nothing
    /// was touched (no pre-flight, no probe, no backup).
    ConfigDisabled,
    /// Pre-flight failed (second lock included: MTU unknown or too small):
    /// nothing was touched.
    Preflight(Vec<PreflightFail>),
    /// The daemon has no complete `0x51`-`0x54` for this connection.
    NoCachedName,
    /// The backup could not be written: nothing was written to the keyboard.
    BackupFailed(String),
    /// Third lock: the typed confirmation did not match; nothing was written.
    Cancelled,
    /// The keyboard disconnected before the write: nothing was written.
    Disconnected,
    /// The session or the register map refused the frame (nothing sent).
    Refused(String),
    /// The write failed (not retried).
    WriteFailed(String),
    /// Written, but the keyboard did not come back in time.
    NoReconnect,
    /// Written and read back identical.
    Verified {
        backup: Option<PathBuf>,
        bluez_name: Option<String>,
    },
    /// Written, read back different: offer the guided restore.
    Mismatch {
        read: Vec<u8>,
        backup: Option<PathBuf>,
        bluez_name: Option<String>,
    },
}

impl Outcome {
    /// True once a frame may have reached the keyboard.
    pub fn wrote(&self) -> bool {
        matches!(
            self,
            Self::WriteFailed(_)
                | Self::NoReconnect
                | Self::Verified { .. }
                | Self::Mismatch { .. }
        )
    }
}

/// The guarded rename (or restore). Order: name, proof, LOCK 1 (configuration),
/// pre-flight with LOCK 2 (control-channel MTU read, ≥ 66), cached bytes,
/// BACKUP (rename only), LOCK 3 (typed confirmation), connection re-checked,
/// ONE write per frame through `session` (each id once), wait for the
/// reconnection, read back, compare. Stops at the first failure, never
/// retries, logs every byte and every decision. `allow_write` is
/// `[apple] allow_device_name_write` as read from `config.toml`.
pub fn run(
    req: &Request,
    proof: SequenceProof,
    allow_write: bool,
    session: &mut WriteSession,
    env: &mut dyn RenameEnv,
) -> Outcome {
    let (frames, target) = match req {
        Request::Rename(n) => match frames_for(n) {
            Ok(f) => (f, n.clone()),
            Err(e) => {
                env.log(&format!("[devname] name refused: {e}"));
                return Outcome::InvalidName(e.to_string());
            }
        },
        Request::Restore(b) => match frames_for_restore(b) {
            Ok(f) => (f, b.name.clone()),
            Err(e) => {
                env.log(&format!("[devname] backup refused: {e}"));
                return Outcome::InvalidName(e);
            }
        },
    };
    if proof != SequenceProof::EstablishedByDisassembly {
        env.log("[devname] decision: real write REFUSED (NotProven): the bytes Apple sends are not established byte for byte (docs/RENOMMER-CLAVIER.md §5.3); nothing was touched");
        return Outcome::NotProven;
    }
    env.log(&format!("[devname] proof: {}", proof.describe()));
    if !allow_write {
        env.log(&format!(
            "[devname] decision: real write REFUSED (lock 1, configuration): [apple] allow_device_name_write is not true; nothing was touched. To lift it:\n{CONFIG_HOWTO}"
        ));
        return Outcome::ConfigDisabled;
    }
    env.log("[devname] lock 1 lifted: [apple] allow_device_name_write = true");
    let pre = env.preflight();
    let fails = preflight(&pre);
    if !fails.is_empty() {
        for f in &fails {
            env.log(&format!("[devname] pre-flight failed: {}", f.describe()));
        }
        return Outcome::Preflight(fails);
    }
    env.log(&format!(
        "[devname] pre-flight ok; lock 2 lifted: control-channel outgoing MTU {} >= {MIN_CONTROL_MTU} (read on the live L2CAP socket)",
        pre.control_mtu.unwrap_or_default()
    ));
    let Some(before) = env.cached_raw().filter(|r| r.len() == MAX_NAME_LEN) else {
        env.log(
            "[devname] decision: stop, the daemon has no complete 0x51-0x54 for this connection",
        );
        return Outcome::NoCachedName;
    };
    let backup_path = if matches!(req, Request::Rename(_)) {
        let mac = env.mac();
        let now = env.now_unix();
        let b = match Backup::new(&mac, &before, now, "daemon-cache") {
            Ok(b) => b,
            Err(e) => {
                env.log(&format!(
                    "[devname] decision: stop, current name not saveable: {e}"
                ));
                return Outcome::BackupFailed(e);
            }
        };
        match env.save_backup(&b) {
            Ok(p) => {
                env.log(&format!(
                    "[devname] backup written: {} (0600), name {:?}",
                    p.display(),
                    b.name
                ));
                Some(p)
            }
            Err(e) => {
                env.log(&format!("[devname] decision: stop, backup failed: {e}"));
                return Outcome::BackupFailed(e.to_string());
            }
        }
    } else {
        None
    };
    let prompt = format!(
        "This writes {} frame(s) into the keyboard's firmware ({}). RISKS NOT MEASURED: the firmware's HANDSHAKE to SET 0x55 (U5) and persistence across a battery change (U3). Type the name exactly ({target:?}) then Enter, anything else cancels: ",
        frames.len(),
        frames.iter().map(|f| format!("{:#04x}", f.id())).collect::<Vec<_>>().join(", ")
    );
    if !env.confirm(&prompt, &target) {
        env.log(
            "[devname] decision: cancelled (lock 3, typed confirmation does not match); nothing written",
        );
        return Outcome::Cancelled;
    }
    env.log("[devname] lock 3 lifted: name typed again");
    if !env.preflight().connected {
        env.log(
            "[devname] decision: stop, the keyboard disconnected before the write; nothing written",
        );
        return Outcome::Disconnected;
    }
    for f in &frames {
        if let Err(e) = session.authorize(f.op, f.id(), Direction::Feature, f.data()) {
            env.log(&format!("[devname] decision: stop, {e}"));
            return Outcome::Refused(e.to_string());
        }
        env.log(&format!(
            "[devname] write {} Feature {:#04x}, {} bytes: {} (wire {})",
            f.op.as_str(),
            f.id(),
            f.report.len(),
            hex(&f.report),
            hex(&f.wire())
        ));
        if let Err(e) = env.sink().set_feature(f.op, &f.report) {
            env.log(&format!(
                "[devname] decision: stop, write failed (not retried): {e}"
            ));
            return Outcome::WriteFailed(e.to_string());
        }
    }
    env.log("[devname] written; waiting for the reconnection (switch the keyboard off and on)");
    let (read, bluez_name) = match env.wait_reconnect(RECONNECT_WAIT) {
        Reconnect::Back { raw, bluez_name } => (raw, bluez_name),
        Reconnect::Timeout => {
            env.log(
                "[devname] decision: stop, the keyboard did not come back in time; no new attempt",
            );
            return Outcome::NoReconnect;
        }
    };
    let expected = expected_readback(&frames);
    env.log(&format!("[devname] read back 0x51-0x54: {}", hex(&read)));
    env.log(&format!(
        "[devname] BlueZ Device1.Name after the reconnection: {} (informative only: Apple checks the HCI remote name, BlueZ may still serve its cache)",
        bluez_name.as_deref().unwrap_or("(unknown)")
    ));
    if read == expected {
        env.log("[devname] verified: the name read back is the one written");
        Outcome::Verified {
            backup: backup_path,
            bluez_name,
        }
    } else {
        env.log("[devname] decision: MISMATCH, offer the guided restore of the backup");
        Outcome::Mismatch {
            read,
            backup: backup_path,
            bluez_name,
        }
    }
}

/// Prepare without writing (`--dry-run`): validate, build the frames. The
/// caller prints them with [`render_frames`].
pub fn prepare(name: &str) -> Result<Vec<Frame>, NameError> {
    frames_for(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Reference of docs/RENOMMER-CLAVIER.md §5.2 for the measured name
    /// "Clavier de maria #1" (construction confirmed by E1).
    const REF_CLAVIER: [u8; 65] = {
        let mut r = [0u8; 65];
        r[0] = 0x55;
        let n = *b"Clavier de maria #1";
        let mut i = 0;
        while i < n.len() {
            r[1 + i] = n[i];
            i += 1;
        }
        r
    };

    /// The measured fragments (docs/HARDWARE-RAPPORTS-HID.md §2).
    fn measured_frames() -> Vec<Vec<u8>> {
        vec![
            [&[0x51u8][..], b"Clavier "].concat(),
            [&[0x52u8][..], b"de maria"].concat(),
            [&[0x53u8][..], b" #1\0\0\0\0\0"].concat(),
            [&[0x54u8][..], &[0u8; 8]].concat(),
        ]
    }

    fn fixture() -> serde_json::Value {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/devname/lion_setdevicename_frames.json");
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
            .unwrap()
    }

    #[test]
    fn validation() {
        assert_eq!(
            validate("Clavier de maria #1").unwrap(),
            "Clavier de maria #1"
        );
        assert_eq!(validate(&"x".repeat(32)).unwrap().len(), 32);
        assert_eq!(
            validate("~!@#$%^&*()_+{}|:<>?`-=[];',./\"").unwrap().len(),
            31
        );
        assert_eq!(validate(""), Err(NameError::Empty));
        assert_eq!(validate(&"x".repeat(33)), Err(NameError::TooLong(33)));
        assert_eq!(validate(" a"), Err(NameError::EdgeSpace));
        assert_eq!(validate("a "), Err(NameError::EdgeSpace));
        assert_eq!(validate(" "), Err(NameError::EdgeSpace));
        assert_eq!(validate("a\tb"), Err(NameError::Control(9)));
        assert_eq!(validate("a\nb"), Err(NameError::Control(10)));
        assert_eq!(validate("a\u{7f}"), Err(NameError::Control(0x7f)));
        assert_eq!(validate("a\0"), Err(NameError::Control(0)));
        assert_eq!(validate("Clavier é"), Err(NameError::NotAscii('é')));
        assert_eq!(validate("a\u{200b}b"), Err(NameError::NotAscii('\u{200b}')));
        assert_eq!(validate("a\\sb"), Err(NameError::Forbidden('\\')));
        for e in [
            NameError::Empty,
            NameError::TooLong(40),
            NameError::TooLongForApple(65),
            NameError::EdgeSpace,
            NameError::Control(1),
            NameError::NotAscii('é'),
            NameError::Forbidden('\\'),
        ] {
            assert!(!e.to_string().is_empty());
        }
        // Every byte value as a one-character name.
        for b in 0u8..=255 {
            let s = String::from(char::from(b));
            let ok = (0x21..=0x7E).contains(&b) && b != b'\\';
            assert_eq!(validate(&s).is_ok(), ok, "{b:#04x}");
        }
    }

    #[test]
    fn apple_encoding_utf8_truncation_by_whole_characters_and_refusals() {
        // ASCII: identity.
        assert_eq!(apple_name_bytes("alex").unwrap(), b"alex");
        assert_eq!(apple_name_bytes(&"a".repeat(64)).unwrap().len(), 64);
        // Refused BEFORE any frame, as Apple: empty, > 64 UTF-16 units.
        assert_eq!(apple_name_bytes(""), Err(NameError::Empty));
        assert_eq!(
            apple_name_bytes(&"a".repeat(65)),
            Err(NameError::TooLongForApple(65))
        );
        // 33 emoji = 66 UTF-16 units (surrogate pairs): refused.
        assert_eq!(
            apple_name_bytes(&"\u{1F600}".repeat(33)),
            Err(NameError::TooLongForApple(66))
        );
        // 40 × "é": 40 units (accepted), 80 UTF-8 bytes → trailing characters
        // dropped to 32 × "é" = 64 bytes, never a split character.
        let e40 = "é".repeat(40);
        let b = apple_name_bytes(&e40).unwrap();
        assert_eq!(b.len(), 64);
        assert_eq!(std::str::from_utf8(&b).unwrap(), "é".repeat(32));
        // 32 emoji: 64 units (accepted), 128 bytes → 16 emoji = 64 bytes.
        let b = apple_name_bytes(&"\u{1F600}".repeat(32)).unwrap();
        assert_eq!(b.len(), 64);
        assert_eq!(std::str::from_utf8(&b).unwrap(), "\u{1F600}".repeat(16));
        // 63 ASCII + "é" = 65 bytes → the "é" is dropped whole: 63 bytes.
        let b = apple_name_bytes(&("a".repeat(63) + "é")).unwrap();
        assert_eq!(b, "a".repeat(63).into_bytes());
        // Every result is valid UTF-8 and ≤ 64 bytes.
        for n in ["a".repeat(64) + "", "日本語".repeat(21), "ü".repeat(64)] {
            let b = apple_name_bytes(&n).unwrap();
            assert!(b.len() <= 64 && std::str::from_utf8(&b).is_ok(), "{n}");
        }
        // The frame of an accepted-but-long name still has 65 bytes, and the
        // tool's own validation (ASCII, ≤ 32) refuses it anyway.
        let f = long_name_frame(&apple_name_bytes(&e40).unwrap());
        assert_eq!(f.report.len(), 65);
        assert!(f.fields.iter().all(|x| x.end <= 65));
        assert_eq!(frames_for(&e40), Err(NameError::NotAscii('é')));
        assert_eq!(frames_for(&"a".repeat(33)), Err(NameError::TooLong(33)));
    }

    #[test]
    fn frames_match_the_lion_fixture_byte_for_byte() {
        let fx = fixture();
        assert_eq!(fx["schema"], 1);
        let r = &fx["report"];
        assert_eq!(r["id"], i64::from(LONG_DEVICE_NAME_ID));
        assert_eq!(r["data_len"], LONG_DEVICE_NAME_LEN as i64);
        assert_eq!(r["userland_len"], 65);
        assert_eq!(r["wire_len"], 66);
        assert_eq!(r["wire_prefix_hex"], "0x53");
        assert_eq!(r["max_name_len_utf16"], APPLE_MAX_NAME_UTF16 as i64);
        assert_eq!(r["max_name_bytes_utf8"], LONG_DEVICE_NAME_LEN as i64);
        assert_eq!(r["length_prefix"], false);
        assert_eq!(fx["sequence"].as_array().unwrap()[0]["wire"], "53 55 <64 octets>");
        let never = fx["never_sent_on_this_path"].as_array().unwrap();
        assert!(never.iter().any(|v| v.as_str().unwrap().starts_with("0x50")));
        assert!(never.iter().any(|v| v.as_str().unwrap().starts_with("0x51-0x54")));
        let examples = fx["examples"].as_array().unwrap();
        assert_eq!(examples.len(), 2);
        for ex in examples {
            let name = ex["name"].as_str().unwrap();
            let frames = frames_for(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1, "{name}: one frame, nothing else");
            let f = &frames[0];
            assert_eq!(f.op, WriteOp::DeviceName);
            assert_eq!(
                f.report,
                unhex(ex["report_hex"].as_str().unwrap()).unwrap(),
                "{name}: report"
            );
            assert_eq!(f.report.len(), 65);
            assert_eq!(
                f.wire(),
                unhex(ex["wire_hex"].as_str().unwrap()).unwrap(),
                "{name}: wire"
            );
            assert_eq!(f.wire().len(), 66);
            assert_eq!(
                expected_readback(&frames),
                unhex(ex["readback_0x51_0x54_expected_hex"].as_str().unwrap()).unwrap(),
                "{name}: read-back"
            );
            assert_eq!(ex["utf8_len"], name.len() as i64);
            // Every byte is covered by a field proven by disassembly.
            let mut covered = [false; 65];
            for fd in &f.fields {
                assert_eq!(fd.proof, Proof::Disassembly, "{name}: {}", fd.what);
                for c in &mut covered[fd.start..fd.end] {
                    *c = true;
                }
            }
            assert!(covered.iter().all(|&c| c), "{name}");
        }
        // The 32-byte example has no padding field left to describe... but
        // still 32 zero bytes after the name (64 data bytes).
        let f = &frames_for("Clavier Apple A1314 du gerant 01").unwrap()[0];
        assert_eq!(&f.report[33..], &[0u8; 32]);
    }

    #[test]
    fn frame_matches_the_documented_reference_byte_for_byte() {
        let f = frames_for("Clavier de maria #1").unwrap();
        assert_eq!(f.len(), 1, "one frame: no 0x50, no 0x51-0x54");
        assert_eq!(f[0].report, REF_CLAVIER.to_vec());
        assert_eq!(f[0].op, WriteOp::DeviceName);
        assert_eq!(f[0].wire()[..3], [0x53, 0x55, b'C']);
        assert_eq!(f[0].wire().len(), 66);
        // The fields cover the 65 bytes, every one with a proof level.
        let mut covered = [false; 65];
        for fd in &f[0].fields {
            for c in &mut covered[fd.start..fd.end] {
                *c = true;
            }
        }
        assert!(covered.iter().all(|&c| c));
        assert!(
            f[0].fields.iter().all(|x| x.proof == Proof::Disassembly),
            "no hypothesis is left in the frame"
        );
        // The read-back of the reference is the measured fragments.
        assert_eq!(
            expected_readback(&f),
            raw_from_frames(&measured_frames()).unwrap()
        );
        let text = render_frames(&f);
        assert!(text.contains("53 55 43 6c 61 76 69 65 72"));
        assert!(text.contains("[désassemblage]") && !text.contains("[hypothèse]"));
        // Full-length names: no padding field beyond 32, still 65 bytes.
        let f = frames_for(&"Z".repeat(32)).unwrap();
        assert_eq!(f[0].report.len(), 65);
        assert_eq!(&f[0].report[33..], &[0u8; 32]);
    }

    #[test]
    fn frames_only_use_the_ids_of_the_operation() {
        for name in ["a", "Clavier de maria #1", &"q".repeat(32)] {
            for f in frames_for(name).unwrap() {
                assert!(f.op.ids().contains(&f.id()));
                assert_eq!(f.data().len(), f.op.payload_len());
                assert!(crate::registry::check_write_op(f.op, f.id(), Direction::Feature).is_ok());
            }
        }
    }

    #[test]
    fn production_proof_is_established_here_and_nowhere_else() {
        assert_eq!(SEQUENCE_PROOF, SequenceProof::EstablishedByDisassembly);
        assert!(SEQUENCE_PROOF.describe().contains("0x4d2fe"));
        assert!(!SequenceProof::NotProven.describe().is_empty());
        assert!(UNKNOWNS.len() >= 3 && RESOLVED_BY_DISASSEMBLY.len() >= 3);
        assert!(UNKNOWNS.iter().filter(|u| u.contains("RISK NOT MEASURED")).count() >= 2);
        assert!(VALIDATION_EXPERIMENTS.iter().any(|e| e.starts_with("E1 DONE")));
        // No production source constructs `SequenceProof::EstablishedByDisassembly`
        // except this module (the constant and the comparison in `run`).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut seen_self = false;
        for d in [
            "akm-core/src",
            "apple-kb-monitord/src",
            "crates/akmctl/src",
            "crates/akm-helper/src",
            "src",
        ] {
            let mut stack = vec![root.join(d)];
            while let Some(p) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&p) else {
                    continue;
                };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                        continue;
                    }
                    if p.extension().is_none_or(|x| x != "rs") {
                        continue;
                    }
                    let text = std::fs::read_to_string(&p).unwrap();
                    let is_self = p.ends_with("akm-core/src/devname.rs");
                    let prod = if is_self {
                        text.split("#[cfg(test)]").next().unwrap().to_string()
                    } else {
                        text
                    };
                    // Code lines only (doc comments may name the variant).
                    let hits = prod
                        .lines()
                        .filter(|l| !l.trim_start().starts_with("//"))
                        .filter(|l| l.contains("SequenceProof::EstablishedByDisassembly"))
                        .count();
                    if is_self {
                        seen_self = true;
                        assert_eq!(hits, 2, "the constant and the comparison in run()");
                    } else {
                        assert_eq!(hits, 0, "{} constructs the proof", p.display());
                    }
                }
            }
        }
        assert!(seen_self);
    }

    #[test]
    fn readback_and_backup_roundtrip() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        assert_eq!(name_from_raw(&raw).unwrap(), "Clavier de maria #1");
        let b = Backup::new("04:DB:56:CA:42:EE", &raw, 1_790_000_000, "daemon-cache").unwrap();
        assert_eq!(b.fragments_hex[0], "436c617669657220");
        assert_eq!(b.raw().unwrap(), raw);
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Backup>(&json).unwrap(), b);
        // Bad inputs.
        let mut f = measured_frames();
        f[2][0] = 0x52;
        assert!(raw_from_frames(&f).is_none(), "ids out of order");
        assert!(raw_from_frames(&measured_frames()[..3]).is_none());
        let mut bad = b.clone();
        bad.name = "autre".into();
        assert!(bad.raw().is_err());
        let mut bad = b.clone();
        bad.fragments_hex[3] = "00ff000000000000".into();
        assert!(bad.raw().is_err(), "garbage after the NUL");
        let mut bad = b.clone();
        bad.fragments_hex[1] = "zz".into();
        assert!(bad.raw().is_err());
        let mut bad = b.clone();
        bad.schema = 9;
        assert!(bad.raw().is_err());
        assert!(Backup::new("m", &raw[..31], 0, "x").is_err());
        assert!(Backup::new("m", &[0xffu8; 32], 0, "x").is_err());
    }

    #[test]
    fn restore_frames_rewrite_exactly_the_backup() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("m", &raw, 0, "daemon-cache").unwrap();
        let f = frames_for_restore(&b).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(&f[0].data()[..32], &raw[..]);
        assert_eq!(&f[0].data()[32..], &[0u8; 32]);
        assert_eq!(f[0].report, REF_CLAVIER.to_vec());
    }

    #[test]
    fn stamps_and_private_files() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
        assert_eq!(utc_stamp(1_790_812_799), "20260930T235959Z");
        assert_eq!(
            backup_file_name("devname", 0),
            "devname-backup-19700101T000000Z.json"
        );
        let dir = std::env::temp_dir().join(format!("akm-devname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("m", &raw, 1_790_812_799, "daemon-cache").unwrap();
        let p = write_backup(&dir, &b).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(read_backup(&p).unwrap(), b);
        assert!(write_backup(&dir, &b).is_err(), "never overwrites a backup");
        std::fs::write(dir.join("bad.json"), "{}").unwrap();
        assert!(read_backup(&dir.join("bad.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn green() -> Preflight {
        Preflight {
            connected: true,
            battery_pct: Some(99.0),
            battery_state_normal: Some(true),
            breaker_open: false,
            recent_read_failure: false,
            doctor_green: true,
            interactive: true,
            control_mtu: Some(672),
        }
    }

    #[test]
    fn preflight_conditions() {
        assert!(preflight(&green()).is_empty());
        let one = |m: fn(&mut Preflight), want: PreflightFail| {
            let mut p = green();
            m(&mut p);
            assert_eq!(preflight(&p), vec![want]);
        };
        one(|p| p.connected = false, PreflightFail::NotConnected);
        one(|p| p.breaker_open = true, PreflightFail::BreakerOpen);
        one(
            |p| p.recent_read_failure = true,
            PreflightFail::RecentReadFailure,
        );
        one(|p| p.doctor_green = false, PreflightFail::DoctorNotGreen);
        one(|p| p.interactive = false, PreflightFail::NotInteractive);
        one(|p| p.control_mtu = None, PreflightFail::ControlMtuUnknown);
        one(
            |p| p.control_mtu = Some(65),
            PreflightFail::ControlMtuTooSmall(65),
        );
        one(
            |p| p.control_mtu = Some(48),
            PreflightFail::ControlMtuTooSmall(48),
        );
        one(
            |p| {
                p.battery_pct = Some(19.9);
                p.battery_state_normal = Some(false)
            },
            PreflightFail::Battery,
        );
        one(
            |p| {
                p.battery_pct = None;
                p.battery_state_normal = None
            },
            PreflightFail::Battery,
        );
        one(
            |p| {
                p.battery_pct = Some(f64::NAN);
                p.battery_state_normal = None
            },
            PreflightFail::Battery,
        );
        // The MTU lock: exactly 66 passes, every value below fails.
        let mut p = green();
        p.control_mtu = Some(MIN_CONTROL_MTU);
        assert!(preflight(&p).is_empty());
        for m in 0..MIN_CONTROL_MTU {
            p.control_mtu = Some(m);
            assert_eq!(preflight(&p), vec![PreflightFail::ControlMtuTooSmall(m)]);
        }
        assert!(PreflightFail::ControlMtuUnknown.is_mtu());
        assert!(PreflightFail::ControlMtuTooSmall(1).is_mtu());
        assert!(!PreflightFail::Battery.is_mtu());
        // Either battery condition is enough.
        let mut p = green();
        p.battery_pct = Some(5.0);
        assert!(preflight(&p).is_empty(), "state normal");
        p.battery_pct = Some(20.0);
        p.battery_state_normal = None;
        assert!(preflight(&p).is_empty(), ">= 20 %");
        assert_eq!(preflight(&Preflight::default()).len(), 5);
        for f in preflight(&Preflight::default()) {
            assert!(!f.describe().is_empty());
        }
    }

    // ── simulated environment ──────────────────────────────────────────────

    #[derive(Default)]
    struct Spy {
        writes: RefCell<Vec<(WriteOp, Vec<u8>)>>,
        events: RefCell<Vec<String>>,
        fail: bool,
    }

    impl FeatureSink for Spy {
        fn set_feature(&self, op: WriteOp, report: &[u8]) -> io::Result<()> {
            self.writes.borrow_mut().push((op, report.to_vec()));
            self.events
                .borrow_mut()
                .push(format!("write {:#04x}", report[0]));
            if self.fail {
                Err(io::Error::from_raw_os_error(libc::EIO))
            } else {
                Ok(())
            }
        }
    }

    struct Sim {
        pre: Preflight,
        /// `connected` seen at the second pre-flight call (just before the write).
        connected_at_write: bool,
        calls: u32,
        cached: Option<Vec<u8>>,
        typed: String,
        backups: Vec<Backup>,
        backup_fails: bool,
        spy: Spy,
        back: Reconnect,
        log: Vec<String>,
    }

    impl Sim {
        fn new(typed: &str) -> Self {
            Self {
                pre: green(),
                connected_at_write: true,
                calls: 0,
                cached: raw_from_frames(&measured_frames()),
                typed: typed.into(),
                backups: Vec::new(),
                backup_fails: false,
                spy: Spy::default(),
                back: Reconnect::Timeout,
                log: Vec::new(),
            }
        }
    }

    fn back(raw: Vec<u8>) -> Reconnect {
        Reconnect::Back {
            raw,
            bluez_name: Some("sim".into()),
        }
    }

    impl RenameEnv for Sim {
        fn preflight(&mut self) -> Preflight {
            self.calls += 1;
            let mut p = self.pre.clone();
            if self.calls > 1 {
                p.connected = self.connected_at_write;
            }
            p
        }
        fn cached_raw(&mut self) -> Option<Vec<u8>> {
            self.cached.clone()
        }
        fn mac(&mut self) -> String {
            "04:DB:56:CA:42:EE".into()
        }
        fn now_unix(&mut self) -> u64 {
            1_790_812_799
        }
        fn save_backup(&mut self, b: &Backup) -> io::Result<PathBuf> {
            self.spy.events.borrow_mut().push("backup".into());
            if self.backup_fails {
                return Err(io::Error::other("disk full"));
            }
            self.backups.push(b.clone());
            Ok(PathBuf::from("/sim/devname-backup.json"))
        }
        fn confirm(&mut self, _prompt: &str, expected: &str) -> bool {
            self.spy.events.borrow_mut().push("confirm".into());
            self.typed == expected
        }
        fn sink(&self) -> &dyn FeatureSink {
            &self.spy
        }
        fn wait_reconnect(&mut self, max: Duration) -> Reconnect {
            assert_eq!(max, RECONNECT_WAIT);
            self.back.clone()
        }
        fn log(&mut self, line: &str) {
            self.log.push(line.to_string());
        }
    }

    /// All three locks lifted: configuration on, MTU 672 (green), name typed.
    fn rename(sim: &mut Sim, name: &str, proof: SequenceProof) -> Outcome {
        run(
            &Request::Rename(name.into()),
            proof,
            true,
            &mut WriteSession::new(),
            sim,
        )
    }

    #[test]
    fn not_proven_refuses_before_touching_anything() {
        let mut sim = Sim::new("Bureau");
        let o = rename(&mut sim, "Bureau", SequenceProof::NotProven);
        assert_eq!(o, Outcome::NotProven);
        assert!(!o.wrote());
        assert!(sim.spy.writes.borrow().is_empty());
        assert!(
            sim.spy.events.borrow().is_empty(),
            "no backup, no confirmation"
        );
        assert_eq!(sim.calls, 0, "not even a pre-flight");
        assert!(sim.log.iter().any(|l| l.contains("NotProven")));
    }

    #[test]
    fn lock_1_configuration_off_refuses_before_any_pre_flight_or_probe() {
        let mut sim = Sim::new("Bureau");
        let o = run(
            &Request::Rename("Bureau".into()),
            SEQUENCE_PROOF,
            false,
            &mut WriteSession::new(),
            &mut sim,
        );
        assert_eq!(o, Outcome::ConfigDisabled);
        assert!(!o.wrote());
        assert_eq!(sim.calls, 0, "no pre-flight, so no MTU probe either");
        assert!(sim.spy.events.borrow().is_empty() && sim.spy.writes.borrow().is_empty());
        let l = sim.log.join("\n");
        assert!(l.contains("lock 1") && l.contains("allow_device_name_write = true"), "{l}");
        // The default of the configuration is the lock closed.
        assert!(!crate::config::Config::default().allow_device_name_write);
        // Restore is behind the same lock.
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("m", &raw, 1, "daemon-cache").unwrap();
        let mut sim = Sim::new("Clavier de maria #1");
        assert_eq!(
            run(&Request::Restore(b), SEQUENCE_PROOF, false, &mut WriteSession::new(), &mut sim),
            Outcome::ConfigDisabled
        );
        assert!(sim.spy.writes.borrow().is_empty());
    }

    #[test]
    fn lock_2_mtu_unknown_or_too_small_refuses_before_backup_and_confirmation() {
        for (mtu, want) in [
            (None, PreflightFail::ControlMtuUnknown),
            (Some(48), PreflightFail::ControlMtuTooSmall(48)),
            (Some(65), PreflightFail::ControlMtuTooSmall(65)),
        ] {
            let mut sim = Sim::new("Bureau");
            sim.pre.control_mtu = mtu;
            let o = rename(&mut sim, "Bureau", SEQUENCE_PROOF);
            assert_eq!(o, Outcome::Preflight(vec![want.clone()]), "{mtu:?}");
            assert!(!o.wrote());
            assert_eq!(sim.calls, 1);
            assert!(
                sim.spy.events.borrow().is_empty(),
                "{mtu:?}: no backup, no confirmation, no write"
            );
            assert!(sim.log.iter().any(|l| l.contains("MTU")), "{mtu:?}");
        }
        // Exactly 66 lifts the lock.
        let mut sim = Sim::new("Bureau");
        sim.pre.control_mtu = Some(MIN_CONTROL_MTU);
        sim.back = back(expected_readback(&frames_for("Bureau").unwrap()));
        assert!(matches!(
            rename(&mut sim, "Bureau", SEQUENCE_PROOF),
            Outcome::Verified { .. }
        ));
        assert!(sim.log.iter().any(|l| l.contains("lock 2 lifted") && l.contains("66")));
    }

    #[test]
    fn lock_3_wrong_confirmation_writes_nothing_after_the_backup() {
        let mut sim = Sim::new("wrong");
        let o = rename(&mut sim, "Bureau", SEQUENCE_PROOF);
        assert_eq!(o, Outcome::Cancelled);
        assert!(!o.wrote());
        assert_eq!(*sim.spy.events.borrow(), vec!["backup", "confirm"]);
        assert!(sim.spy.writes.borrow().is_empty());
        assert!(sim.log.iter().any(|l| l.contains("lock 3")));
    }

    #[test]
    fn three_locks_lifted_the_fixture_frame_is_sent_once_after_the_backup() {
        let fx = fixture();
        for ex in fx["examples"].as_array().unwrap() {
            let name = ex["name"].as_str().unwrap();
            let want = unhex(ex["report_hex"].as_str().unwrap()).unwrap();
            let mut sim = Sim::new(name);
            sim.back = back(unhex(ex["readback_0x51_0x54_expected_hex"].as_str().unwrap()).unwrap());
            let o = rename(&mut sim, name, SEQUENCE_PROOF);
            assert_eq!(
                o,
                Outcome::Verified {
                    backup: Some("/sim/devname-backup.json".into()),
                    bluez_name: Some("sim".into()),
                },
                "{name}"
            );
            // Order: backup, confirmation, then the single write.
            assert_eq!(
                *sim.spy.events.borrow(),
                vec!["backup", "confirm", "write 0x55"],
                "{name}"
            );
            let w = sim.spy.writes.borrow();
            assert_eq!(w.len(), 1, "{name}: exactly one frame");
            assert_eq!(w[0].0, WriteOp::DeviceName);
            assert_eq!(w[0].1, want, "{name}: the fixture bytes, exactly");
            assert_eq!(w[0].1.len(), 65);
            assert_eq!(sim.backups[0].name, "Clavier de maria #1");
            // Every byte sent is in the journal, with the wire form.
            assert!(sim.log.iter().any(|l| l.contains(&hex(&w[0].1))));
            assert!(sim.log.iter().any(|l| l.contains(&format!("wire 53 {}", hex(&w[0].1)))));
            assert_eq!(sim.calls, 2, "pre-flight, then the connection re-check");
        }
    }

    #[test]
    fn reference_frame_reaches_the_spy_byte_for_byte() {
        let mut sim = Sim::new("Clavier de maria #1");
        sim.back = back(raw_from_frames(&measured_frames()).unwrap());
        assert!(matches!(
            rename(&mut sim, "Clavier de maria #1", SEQUENCE_PROOF),
            Outcome::Verified { .. }
        ));
        assert_eq!(sim.spy.writes.borrow()[0].1, REF_CLAVIER.to_vec());
    }

    #[test]
    fn refusals_send_nothing() {
        type Case<'a> = (Box<dyn Fn(&mut Sim)>, &'a str, fn(&Outcome) -> bool);
        let cases: Vec<Case> = vec![
            (Box::new(|_| {}), "a\\b", |o| {
                matches!(o, Outcome::InvalidName(_))
            }),
            (Box::new(|_| {}), "x", |o| matches!(o, Outcome::Cancelled)), // typed "wrong"
            (Box::new(|s| s.pre.connected = false), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.breaker_open = true), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.recent_read_failure = true), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.doctor_green = false), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.interactive = false), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.control_mtu = None), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.control_mtu = Some(60)), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (
                Box::new(|s| {
                    s.pre.battery_pct = Some(3.0);
                    s.pre.battery_state_normal = Some(false)
                }),
                "x",
                |o| matches!(o, Outcome::Preflight(_)),
            ),
            (Box::new(|s| s.cached = None), "x", |o| {
                *o == Outcome::NoCachedName
            }),
            (Box::new(|s| s.backup_fails = true), "x", |o| {
                matches!(o, Outcome::BackupFailed(_))
            }),
            (Box::new(|s| s.connected_at_write = false), "x", |o| {
                *o == Outcome::Disconnected
            }),
        ];
        for (i, (setup, name, want)) in cases.into_iter().enumerate() {
            let mut sim = Sim::new(if i == 1 { "wrong" } else { "x" });
            setup(&mut sim);
            let o = rename(&mut sim, name, SEQUENCE_PROOF);
            assert!(want(&o), "case {i}: {o:?}");
            assert!(!o.wrote(), "case {i}");
            assert!(
                sim.spy.writes.borrow().is_empty(),
                "case {i}: nothing may be written"
            );
        }
        // A backup failure stops before the confirmation.
        let mut sim = Sim::new("x");
        sim.backup_fails = true;
        rename(&mut sim, "x", SEQUENCE_PROOF);
        assert_eq!(*sim.spy.events.borrow(), vec!["backup"]);
    }

    #[test]
    fn a_used_session_refuses_a_second_write() {
        let mut s = WriteSession::new();
        let mut sim = Sim::new("x");
        sim.back = back(expected_readback(&frames_for("x").unwrap()));
        assert!(matches!(
            run(
                &Request::Rename("x".into()),
                SEQUENCE_PROOF,
                true,
                &mut s,
                &mut sim
            ),
            Outcome::Verified { .. }
        ));
        let mut sim2 = Sim::new("y");
        let o = run(
            &Request::Rename("y".into()),
            SEQUENCE_PROOF,
            true,
            &mut s,
            &mut sim2,
        );
        assert!(
            matches!(o, Outcome::Refused(ref e) if e.contains("already sent")),
            "{o:?}"
        );
        assert!(sim2.spy.writes.borrow().is_empty());
    }

    #[test]
    fn failure_mismatch_and_timeout_stop_without_retry() {
        let mut sim = Sim::new("x");
        sim.spy.fail = true;
        let o = rename(&mut sim, "x", SEQUENCE_PROOF);
        assert!(matches!(o, Outcome::WriteFailed(_)) && o.wrote());
        assert_eq!(sim.spy.writes.borrow().len(), 1, "one attempt only");

        let mut sim = Sim::new("x");
        let o = rename(&mut sim, "x", SEQUENCE_PROOF);
        assert_eq!(o, Outcome::NoReconnect);
        assert_eq!(sim.spy.writes.borrow().len(), 1);

        let mut sim = Sim::new("x");
        sim.back = back(vec![0; 32]);
        let o = rename(&mut sim, "x", SEQUENCE_PROOF);
        assert!(matches!(o, Outcome::Mismatch { ref backup, .. } if backup.is_some()));
        assert_eq!(sim.spy.writes.borrow().len(), 1);
        assert!(sim.log.iter().any(|l| l.contains("Device1.Name") && l.contains("informative")));
    }

    #[test]
    fn restore_rewrites_exactly_the_backup_with_a_new_confirmation_and_no_new_backup() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("04:DB:56:CA:42:EE", &raw, 1, "daemon-cache").unwrap();
        let mut sim = Sim::new("Clavier de maria #1");
        sim.cached = Some(expected_readback(&frames_for("x").unwrap()));
        sim.back = back(raw.clone());
        let o = run(
            &Request::Restore(b.clone()),
            SEQUENCE_PROOF,
            true,
            &mut WriteSession::new(),
            &mut sim,
        );
        assert_eq!(
            o,
            Outcome::Verified {
                backup: None,
                bluez_name: Some("sim".into())
            }
        );
        assert_eq!(*sim.spy.events.borrow(), vec!["confirm", "write 0x55"]);
        let w = sim.spy.writes.borrow();
        assert_eq!(&w[0].1[1..33], &raw[..]);
        assert_eq!(&w[0].1[33..], &[0u8; 32]);
        // Not confirmed: nothing.
        let mut sim = Sim::new("no");
        assert_eq!(
            run(
                &Request::Restore(b.clone()),
                SEQUENCE_PROOF,
                true,
                &mut WriteSession::new(),
                &mut sim
            ),
            Outcome::Cancelled
        );
        assert!(sim.spy.writes.borrow().is_empty());
        // Restore is refused as well while the sequence is not proven.
        let mut sim = Sim::new("x");
        assert_eq!(
            run(
                &Request::Restore(b),
                SequenceProof::NotProven,
                true,
                &mut WriteSession::new(),
                &mut sim
            ),
            Outcome::NotProven
        );
    }
}
