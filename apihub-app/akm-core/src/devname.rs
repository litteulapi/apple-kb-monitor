//! The name stored **in** the keyboard, as opposed to the alias of this computer,
//! which stays the default way to rename.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::conv::hex_compact;
use crate::parity::FeatureSink;
use crate::registry::{Direction, Proof, WriteOp, WriteSession};
use crate::tr;

/// `LongDeviceName` report id \[plist\] [disassembly `0x4d444`].
pub const LONG_DEVICE_NAME_ID: u8 = 0x55;
/// Data bytes of `LongDeviceName` \[plist\]; the report handed to the kernel is 65 bytes.
pub const LONG_DEVICE_NAME_LEN: usize = 64;
/// Longest name Apple's UI and `setDeviceName:` accept, in UTF-16 units [disassembly `0x4d3d3`].
#[cfg(test)]
pub const APPLE_MAX_NAME_UTF16: usize = 64;
/// `DeviceName1..4`, read back right after the write \[plist\] \[measured\].
pub const FRAGMENT_IDS: [u8; 4] = [0x51, 0x52, 0x53, 0x54];
/// Data bytes of each fragment \[plist\] \[measured\].
pub const FRAGMENT_LEN: usize = 8;
/// Longest name accepted here.
pub const MAX_NAME_LEN: usize = FRAGMENT_IDS.len() * FRAGMENT_LEN;
/// Timeout Apple passes to `IOHIDDeviceInterface::setReport` (1000 ms) [disassembly `0x4d4ba`].
pub const APPLE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);
/// Smallest outgoing L2CAP MTU of the HIDP control channel that carries the 65-byte report in one
/// frame: `0x53` + 65 [disassembly `setReportWL` `0x5ad6`: chunk = MTU − 1].
pub const MIN_CONTROL_MTU: u16 = 66;
/// Minimum battery for a write (else the keyboard must report state `normal`).
pub const MIN_BATTERY_PCT: f64 = 20.0;
/// Subdirectory of `$XDG_STATE_HOME` holding the backups.
pub const STATE_SUBDIR: &str = "apple-kb-monitor";
/// Characters refused although printable ASCII.
pub const FORBIDDEN: &[char] = &['\\'];
/// Proof of the byte sequence Apple sends: frame construction read in Apple's own code
/// (docs/RENAME-KEYBOARD.md).
pub const SEQUENCE_PROOF: &str = "established by disassembly (E1): IOBluetooth 10.7.5 x86_64 -[AppleBluetoothHIDDevice setDeviceName:] 0x4d2fe, identical in 10.5.8 i386 0x4fc10; reference frames tests/fixtures/devname/lion_setdevicename_frames.json";

/// Settled by E1 (docs/RENAME-KEYBOARD.md §3).
pub const RESOLVED_BY_DISASSEMBLY: &[&str] = &[
    "U1 content of the 64 bytes: UTF-8 name (trailing characters dropped until <= 64 bytes), 0x00 padding (calloc), no length prefix, no explicit terminator (a 64-byte name has no NUL) [disassembly 0x4d3fe-0x4d444]",
    "U2 order: ONE SET Feature 0x55 of 65 bytes, then an HCI Remote Name Request; no GET before or after, no 0x50, no 0x51-0x54, no 0x44, no 0x41; empty or > 64 UTF-16 units refused before any frame (kIOReturnBadArgument) [disassembly 0x4d3d3-0x4d4c8]",
    "U7 0x50 DeviceNameChange validates the 4-fragment path only (PIDs without LongDeviceName); never sent to this keyboard [disassembly 0x4d525-0x4d6cb]",
    "U6 (Apple side) one frame of 66 bytes when the control channel's outgoing MTU >= 66, else DATC fragments; Linux side: the MTU is READ at pre-flight (akm-helper hid-inspect, getsockopt L2CAP_OPTIONS, read-only) and the write is refused below 66 or when unknown",
];

/// Settled by measurement on the real keyboard (firmware `0x0050`), 02/10/2026.
pub const RESOLVED_BY_MEASUREMENT: &[&str] = &[
    "U5 [measured 2026-10-02] SET Feature 0x55 (65 bytes) is ACCEPTED by firmware 0x0050: the write succeeds (HANDSHAKE SUCCESSFUL); the link did not move (breaker at 0), outgoing MTU of the control channel = 185",
    "U4 [measured 2026-10-02] 0x51-0x54 reflect 0x55 IN THE SAME CONNECTION, without switching the keyboard off: read ~75 s after the write, the name written + 0x00 padding over 32 bytes; no reconnection is needed (BlueZ Device1.Name keeps its cached name until a later connection)",
];

/// What remains unmeasured: hardware facts no disassembly can give.
pub const UNKNOWNS: &[&str] = &[
    "U3 NOT MEASURED: persistence across a power-off or a battery change (the Magic Keyboard descriptor declares its 0x55 Feature 'volatile'); Apple never rewrites the name at reconnection, so the keyboard is expected to store it (deduction, E3); the backup allows rewriting it",
];

/// Experiments: done, or that would lift the remaining unknown.
pub const VALIDATION_EXPERIMENTS: &[&str] = &[
    "E1 DONE (U1, U2, U7, U6 Apple side): docs/RENAME-KEYBOARD.md, fixture tests/fixtures/devname/lion_setdevicename_frames.json",
    "E2 (U6 Linux side) is a pre-flight: `akm-helper hid-inspect --mac <MAC>` reads the negotiated L2CAP MTU of the control channel (read-only) and the write is refused below 66",
    "E5 DONE [measured 2026-10-02] (U4, U5): real write on firmware 0x0050, accepted, read back identical in the same connection",
    "E3 (U3) compare the cached 0x51-0x54, HID_NAME and BlueZ Name before and after a battery change (daemon cache, no new read)",
];

/// Why a name is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong(usize),
    /// More than [`APPLE_MAX_NAME_UTF16`] UTF-16 units (Apple's model, never reached past [`validate`]).
    #[cfg(test)]
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
            Self::Empty => f.write_str(&tr!("empty name")),
            Self::TooLong(n) => f.write_str(&tr!(
                "{n} characters, at most {max} (what 0x51-0x54 can show back)",
                n = n,
                max = MAX_NAME_LEN
            )),
            #[cfg(test)]
            Self::TooLongForApple(n) => write!(
                f,
                "{n} UTF-16 units, Apple refuses above {APPLE_MAX_NAME_UTF16} before any frame (kIOReturnBadArgument)"
            ),
            Self::EdgeSpace => f.write_str(&tr!("leading or trailing space")),
            Self::Control(c) => f.write_str(&tr!("control character U+{code}", code = format!("{c:04X}"))),
            Self::NotAscii(c) => f.write_str(&tr!(
                "non-ASCII character {c} (the keyboard stores ASCII)",
                c = format!("{c:?}")
            )),
            Self::Forbidden(c) => f.write_str(&tr!(
                "character {c} refused (escape character of BlueZ's key file)",
                c = format!("{c:?}")
            )),
        }
    }
}

impl std::error::Error for NameError {}

/// Validate a new name: printable ASCII (`0x20`-`0x7E`), 1 to [`MAX_NAME_LEN`] characters, no space
/// at either end, none of [`FORBIDDEN`].
///
/// # Errors
///
/// [`NameError`] naming the rule the name breaks.
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

/// The name bytes exactly as Apple builds them (documented model; production sends [`validate`]d
/// ASCII, which it leaves unchanged).
#[cfg(test)]
fn apple_name_bytes(name: &str) -> Result<Vec<u8>, NameError> {
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
    #[must_use]
    pub fn id(&self) -> u8 {
        self.report[0]
    }
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.report[1..]
    }
    /// Bytes on the HIDP control channel.
    #[must_use]
    pub fn wire(&self) -> Vec<u8> {
        let mut w = vec![0x53];
        w.extend_from_slice(&self.report);
        w
    }
}

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
            what: "report id 0x55 LongDeviceName, the only report Lion's setDeviceName: writes for a PID that declares it [plist + disassembly 0x4d444 `movb %bl,(%r13)`]",
        },
        Field {
            start: 1,
            end: 1 + LONG_DEVICE_NAME_LEN,
            proof: Proof::Disassembly,
            what: "64 data bytes (report of 65): size of LongDeviceName in the 598 personality, getMaxDeviceNameLength = 64, setReport length 65 [plist + disassembly 0x4d4b1]",
        },
    ];
    if n > 0 {
        fields.push(Field {
            start: 1,
            end: 1 + n,
            proof: Proof::Disassembly,
            what: "name as UTF-8 bytes from byte 1, whole characters, no terminator [disassembly 0x4d416 _UTF8StringFromString, 0x4d43f strncpy(buf+1, utf8, strlen)]",
        });
    }
    if n < LONG_DEVICE_NAME_LEN {
        fields.push(Field {
            start: 1 + n,
            end: 1 + LONG_DEVICE_NAME_LEN,
            proof: Proof::Disassembly,
            what: "0x00 padding to 64 bytes: calloc(65, 1), nothing else written [disassembly 0x4d407]",
        });
    }
    Frame {
        op: WriteOp::DeviceName,
        report,
        fields,
    }
}

/// The frames of Apple's rename for a validated `name`.
///
/// # Errors
///
/// [`NameError`] when `name` is not valid (see [`validate`]).
pub fn frames_for(name: &str) -> Result<Vec<Frame>, NameError> {
    let name = validate(name)?;
    Ok(vec![long_name_frame(name.as_bytes())])
}

/// The frames that rewrite a backup: the 32 saved bytes exactly, then NUL.
///
/// # Errors
///
/// A message when the backup fails its checks (see [`Backup::raw`]).
pub fn frames_for_restore(b: &Backup) -> Result<Vec<Frame>, String> {
    let raw = b.raw()?;
    Ok(vec![long_name_frame(&raw)])
}

/// The 32 bytes `0x51`-`0x54` should read back after `frames`.
#[must_use]
pub fn expected_readback(frames: &[Frame]) -> Vec<u8> {
    frames
        .iter()
        .find(|f| f.id() == LONG_DEVICE_NAME_ID)
        .map(|f| f.data()[..MAX_NAME_LEN].to_vec())
        .unwrap_or_default()
}

/// Hex with spaces.
#[must_use]
pub fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bytes of a hex string (spaces allowed); `None` if malformed.
#[must_use]
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
#[must_use]
pub fn render_frames(frames: &[Frame]) -> String {
    let mut s = String::new();
    for (i, f) in frames.iter().enumerate() {
        let _ = writeln!(s,
            "frame {}/{} — operation {}, SET Feature {:#04x}, {} byte(s) handed to the kernel\n  report : {}\n  wire   : {}",
            i + 1,
            frames.len(),
            f.op.as_str(),
            f.id(),
            f.report.len(),
            hex(&f.report),
            hex(&f.wire()),
        );
        for fd in &f.fields {
            let _ = writeln!(
                s,
                "  bytes {:>2}..{:<2} {:<16} {}",
                fd.start,
                fd.end,
                fd.proof.tag(),
                fd.what
            );
        }
    }
    let _ = writeln!(
        s,
        "then: no other frame (no 0x50, no 0x51-0x54, no read) [disassembly]; HANDSHAKE awaited at most {} ms [disassembly]; \
         Apple then asks the HCI remote name; here the name is read back (0x51-0x54) at once, in the same connection [measured]",
        APPLE_HANDSHAKE_TIMEOUT.as_millis()
    );
    s
}

/// The 32 raw bytes of `0x51`-`0x54` from four frames (id first, as `HIDIOCGFEATURE` returns them).
#[must_use]
pub fn raw_from_frames(frames: &[Vec<u8>]) -> Option<Vec<u8>> {
    if frames.len() != FRAGMENT_IDS.len() {
        return None;
    }
    let mut raw = Vec::with_capacity(MAX_NAME_LEN);
    for (f, id) in frames.iter().zip(FRAGMENT_IDS) {
        if f.len() < 1 + FRAGMENT_LEN || f[0] != id {
            return None;
        }
        raw.extend_from_slice(&f[1..=FRAGMENT_LEN]);
    }
    Some(raw)
}

/// The name in 32 raw bytes: up to the first NUL; `None` if not printable ASCII.
#[must_use]
pub fn name_from_raw(raw: &[u8]) -> Option<String> {
    let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let s = &raw[..n];
    s.iter()
        .all(|b| (0x20..=0x7E).contains(b))
        .then(|| String::from_utf8_lossy(s).into_owned())
}

/// A name to SHOW for 32 raw bytes, whatever they hold.
#[must_use]
pub fn display_name_from_raw(raw: &[u8]) -> String {
    let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..n])
        .chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

/// The node the door opened must be the keyboard the pre-flight, the MTU probe and the backup were
/// made for.
///
/// # Errors
///
/// A message when the two MAC addresses differ.
pub fn check_door_mac(door_mac: &str, expected: &str) -> Result<(), String> {
    let known = |m: &str| m.len() == 17 && m.split(':').count() == 6;
    if known(door_mac) && known(expected) && door_mac.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    Err(format!(
        "the hidraw node opened belongs to keyboard {} while the pre-flight and the backup concern {}",
        if door_mac.is_empty() { "(address unknown)" } else { door_mac },
        if expected.is_empty() { "(address unknown)" } else { expected },
    ))
}

/// Read `0x51`-`0x54` through `src` and return the 32 data bytes.
///
/// # Errors
///
/// The I/O error of a read, or `InvalidData` for a malformed report.
pub fn read_name_from(src: &dyn crate::decode::HidSource) -> io::Result<Vec<u8>> {
    let mut raw = Vec::with_capacity(MAX_NAME_LEN);
    for id in FRAGMENT_IDS {
        let f = src.feature(id)?;
        if f.len() != 1 + FRAGMENT_LEN || f[0] != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "report {id:#04x}: {} byte(s) [{}], expected the id + {FRAGMENT_LEN}",
                    f.len(),
                    hex(&f)
                ),
            ));
        }
        raw.extend_from_slice(&f[1..]);
    }
    Ok(raw)
}

/// The door a short-lived command opens for the name.
pub trait NameDoor: FeatureSink {
    /// Read the 32 data bytes of the current name.
    ///
    /// # Errors
    ///
    /// The I/O error of the read.
    fn read_name(&self) -> io::Result<Vec<u8>>;
    fn as_sink(&self) -> &dyn FeatureSink;
    fn mac(&self) -> String;
}

/// What was in `0x51`-`0x54` before any write: the 32 bytes as they are, whatever they hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    pub schema: u32,
    /// Unix time of the backup.
    pub created_unix: u64,
    pub mac: String,
    /// The four fragments, 8 data bytes each, hex (id not included).
    pub fragments_hex: [String; 4],
    /// The name they spell when it is printable ASCII, else `None`.
    #[serde(default)]
    pub name: Option<String>,
    /// Where the bytes come from.
    pub source: String,
}

impl Backup {
    pub const SCHEMA: u32 = 1;

    /// Save 32 raw bytes. Never refused for their content.
    ///
    /// # Errors
    ///
    /// A message when `raw` is not 32 bytes long.
    pub fn new(mac: &str, raw: &[u8], created_unix: u64, source: &str) -> Result<Self, String> {
        if raw.len() != MAX_NAME_LEN {
            return Err(format!("{} bytes, expected {MAX_NAME_LEN}", raw.len()));
        }
        let frag = |i: usize| hex_compact(&raw[i * FRAGMENT_LEN..(i + 1) * FRAGMENT_LEN]);
        Ok(Self {
            schema: Self::SCHEMA,
            created_unix,
            mac: mac.to_string(),
            fragments_hex: [frag(0), frag(1), frag(2), frag(3)],
            name: name_from_raw(raw),
            source: source.to_string(),
        })
    }

    /// The saved name for the user (lossy for bytes that are not ASCII).
    #[must_use]
    pub fn display_name(&self) -> String {
        if let Some(n) = &self.name {
            n.clone()
        } else {
            let raw: Vec<u8> = self
                .fragments_hex
                .iter()
                .filter_map(|h| unhex(h))
                .flatten()
                .collect();
            display_name_from_raw(&raw)
        }
    }

    /// The 32 saved bytes, checked for what makes a backup trustworthy.
    ///
    /// # Errors
    ///
    /// A message naming the check that fails.
    pub fn raw(&self) -> Result<Vec<u8>, String> {
        if self.schema != Self::SCHEMA {
            return Err(tr!("backup schema {schema} unknown", schema = self.schema));
        }
        let mut raw = Vec::with_capacity(MAX_NAME_LEN);
        for (i, h) in self.fragments_hex.iter().enumerate() {
            let b = unhex(h).ok_or_else(|| tr!("fragment {i} is not hex", i = i + 1))?;
            if b.len() != FRAGMENT_LEN {
                return Err(tr!(
                    "fragment {i} has {n} bytes, expected {expected}",
                    i = i + 1,
                    n = b.len(),
                    expected = FRAGMENT_LEN
                ));
            }
            raw.extend_from_slice(&b);
        }
        let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        if raw[n..].iter().any(|&b| b != 0) {
            return Err(tr!(
                "bytes after the NUL terminator: refused (not a name this tool restores)"
            ));
        }
        if n == 0 {
            return Err(tr!("the saved name is empty: refused"));
        }
        if name_from_raw(&raw) != self.name {
            return Err(tr!("the fragments do not spell the saved name"));
        }
        Ok(raw)
    }
}

/// `$XDG_STATE_HOME/apple-kb-monitor` (fallback `~/.local/state/...`).
#[must_use]
pub fn state_dir() -> PathBuf {
    crate::paths::state_home().join(STATE_SUBDIR)
}

/// `YYYYMMDDTHHMMSSZ` of a Unix time (UTC).
#[must_use]
pub fn utc_stamp(unix: u64) -> String {
    let (y, m, d, rem) = crate::conv::civil_utc(unix);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[must_use]
pub fn backup_file_name(prefix: &str, unix: u64) -> String {
    format!("{prefix}-backup-{}.json", utc_stamp(unix))
}

/// Write `json` to a NEW file `dir/name`, mode 0600 (directory 0700), synced to disk.
///
/// # Errors
///
/// Any I/O error, `AlreadyExists` included.
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

/// Write `b` to a new private file in `dir`.
///
/// # Errors
///
/// Any I/O error from [`write_private_file`].
pub fn write_backup(dir: &Path, b: &Backup) -> io::Result<PathBuf> {
    let json = serde_json::to_string_pretty(b).map_err(io::Error::other)?;
    write_private_file(dir, &backup_file_name("devname", b.created_unix), &json)
}

/// Read and check a backup file.
///
/// # Errors
///
/// A message when the file cannot be read, parsed or checked.
pub fn read_backup(path: &Path) -> Result<Backup, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let b: Backup = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    b.raw()?;
    Ok(b)
}

/// Facts gathered before a write (from the daemon, `akmctl doctor` and the read-only MTU probe).
#[derive(Debug, Clone, Default, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // reason: independent pre-flight facts, serialized as is
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
    /// Negotiated **outgoing** MTU of the L2CAP control channel to this keyboard, read on the live
    /// socket; `None` = unknown = refused.
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
    /// The MTU of the control channel could not be read.
    ControlMtuUnknown,
    /// The MTU is known and below [`MIN_CONTROL_MTU`].
    ControlMtuTooSmall(u16),
}

impl PreflightFail {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::NotConnected => tr!("keyboard not connected"),
            Self::Battery => tr!("battery below 20 % and state not 'normal' (or unknown)"),
            Self::BreakerOpen => tr!("circuit breaker open: the keyboard stopped answering"),
            Self::RecentReadFailure => tr!("the last hardware read failed or was incomplete"),
            Self::DoctorNotGreen => {
                tr!("`akmctl doctor` is not green (link health, pairing, configuration)")
            }
            Self::ControlMtuUnknown => tr!(
                "outgoing MTU of the L2CAP control channel unknown: it must be read (akm-helper hid-inspect, read-only) and be >= {min} before a 66-byte frame is sent",
                min = MIN_CONTROL_MTU
            ),
            Self::ControlMtuTooSmall(m) => tr!(
                "outgoing MTU of the L2CAP control channel is {m}, below {min}: a 66-byte frame cannot be sent in one piece (Linux hidp does not fragment; the HID session would be terminated)",
                m = m,
                min = MIN_CONTROL_MTU
            ),
        }
    }

    /// Is this the MTU condition (as opposed to the daemon's facts)?
    #[must_use]
    pub fn is_mtu(&self) -> bool {
        matches!(self, Self::ControlMtuUnknown | Self::ControlMtuTooSmall(_))
    }
}

/// Every failed condition (empty = go).
#[must_use]
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
    match p.control_mtu {
        None => v.push(PreflightFail::ControlMtuUnknown),
        Some(m) if m < MIN_CONTROL_MTU => v.push(PreflightFail::ControlMtuTooSmall(m)),
        Some(_) => {}
    }
    v
}

/// Everything [`run`] needs from the outside world (simulated in the tests).
pub trait RenameEnv {
    fn preflight(&mut self) -> Preflight;
    fn cached_raw(&mut self) -> Option<Vec<u8>>;
    fn mac(&mut self) -> String;
    fn now_unix(&mut self) -> u64;
    /// Save the backup before any write.
    ///
    /// # Errors
    ///
    /// Any I/O error; the rename is then not attempted.
    fn save_backup(&mut self, b: &Backup) -> io::Result<PathBuf>;
    fn confirm(&mut self, target: &str) -> bool;
    fn sink(&self) -> &dyn FeatureSink;
    /// Read the name back after the write.
    ///
    /// # Errors
    ///
    /// The I/O error of the read.
    fn read_back(&mut self) -> io::Result<Vec<u8>>;
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
    /// Pre-flight failed (MTU unknown or too small included): nothing was touched.
    Preflight(Vec<PreflightFail>),
    /// The daemon has no complete `0x51`-`0x54` for this connection.
    NoCachedName,
    /// The backup could not be written: nothing was written to the keyboard.
    BackupFailed(String),
    /// The backup belongs to another keyboard than the target: nothing was touched.
    OtherKeyboard { backup: String, target: String },
    /// The answer to the confirmation was not a yes; nothing was written.
    Cancelled,
    /// The keyboard disconnected before the write: nothing was written.
    Disconnected,
    /// The session or the register map refused the frame (nothing sent).
    Refused(String),
    /// The write failed (not retried).
    WriteFailed(String),
    /// Written and read back identical.
    Verified { backup: Option<PathBuf> },
    /// Written, read back different: offer the restore of the backup.
    Mismatch {
        read: Vec<u8>,
        backup: Option<PathBuf>,
    },
    /// Written, but the read-back was not possible (not retried).
    Unverified {
        backup: Option<PathBuf>,
        error: String,
    },
}

impl Outcome {
    /// True once a frame may have reached the keyboard.
    #[must_use]
    pub fn wrote(&self) -> bool {
        matches!(
            self,
            Self::WriteFailed(_)
                | Self::Verified { .. }
                | Self::Mismatch { .. }
                | Self::Unverified { .. }
        )
    }
}

/// Save the current name before a rename or a restore.
fn save_current_name(env: &mut dyn RenameEnv, before: &[u8]) -> Result<PathBuf, Outcome> {
    let mac = env.mac();
    let now = env.now_unix();
    let b = match Backup::new(&mac, before, now, "daemon-cache") {
        Ok(b) => b,
        Err(e) => {
            env.log(&format!(
                "[devname] decision: stop, current name not saveable: {e}"
            ));
            return Err(Outcome::BackupFailed(e));
        }
    };
    match env.save_backup(&b) {
        Ok(p) => {
            env.log(&format!(
                "[devname] backup written: {} (0600), name {:?}",
                p.display(),
                b.display_name()
            ));
            Ok(p)
        }
        Err(e) => {
            env.log(&format!("[devname] decision: stop, backup failed: {e}"));
            Err(Outcome::BackupFailed(e.to_string()))
        }
    }
}

/// Authorise then send each frame; stop at the first refusal or failure (never retried).
fn write_frames(
    frames: &[Frame],
    session: &mut WriteSession,
    env: &mut dyn RenameEnv,
) -> Result<(), Outcome> {
    for f in frames {
        if let Err(e) = session.authorize(f.op, f.id(), Direction::Feature, f.data()) {
            env.log(&format!("[devname] decision: stop, {e}"));
            return Err(Outcome::Refused(e.to_string()));
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
            return Err(Outcome::WriteFailed(e.to_string()));
        }
    }
    Ok(())
}

/// The guarded rename (or restore).
pub fn run(req: &Request, session: &mut WriteSession, env: &mut dyn RenameEnv) -> Outcome {
    let (frames, target) = match req {
        Request::Rename(n) => match frames_for(n) {
            Ok(f) => (f, n.clone()),
            Err(e) => {
                env.log(&format!("[devname] name refused: {e}"));
                return Outcome::InvalidName(e.to_string());
            }
        },
        Request::Restore(b) => match frames_for_restore(b) {
            Ok(f) => (f, b.display_name()),
            Err(e) => {
                env.log(&format!("[devname] backup refused: {e}"));
                return Outcome::InvalidName(e);
            }
        },
    };
    if let Request::Restore(b) = req {
        let target = env.mac();
        if check_door_mac(&b.mac, &target).is_err() {
            env.log(&format!(
                "[devname] decision: stop, the backup is of keyboard {} and the target is {target}; nothing was touched",
                b.mac
            ));
            return Outcome::OtherKeyboard {
                backup: b.mac.clone(),
                target,
            };
        }
    }
    env.log(&format!("[devname] proof: {SEQUENCE_PROOF}"));
    let pre = env.preflight();
    let fails = preflight(&pre);
    if !fails.is_empty() {
        for f in &fails {
            env.log(&format!("[devname] pre-flight failed: {}", f.describe()));
        }
        return Outcome::Preflight(fails);
    }
    env.log(&format!(
        "[devname] pre-flight ok: control-channel outgoing MTU {} >= {MIN_CONTROL_MTU} (read on the live L2CAP socket)",
        pre.control_mtu.unwrap_or_default()
    ));
    let Some(before) = env.cached_raw().filter(|r| r.len() == MAX_NAME_LEN) else {
        env.log(
            "[devname] decision: stop, the daemon has no complete 0x51-0x54 for this connection",
        );
        return Outcome::NoCachedName;
    };
    let backup_path = match save_current_name(env, &before) {
        Ok(p) => Some(p),
        Err(o) => return o,
    };
    if !env.confirm(&target) {
        env.log("[devname] decision: cancelled (not confirmed); nothing written");
        return Outcome::Cancelled;
    }
    env.log("[devname] confirmed");
    if !env.preflight().connected {
        env.log(
            "[devname] decision: stop, the keyboard disconnected before the write; nothing written",
        );
        return Outcome::Disconnected;
    }
    if let Err(o) = write_frames(&frames, session, env) {
        return o;
    }
    env.log("[devname] written; reading 0x51-0x54 back in the same connection");
    let read = match env.read_back() {
        Ok(r) => r,
        Err(e) => {
            env.log(&format!(
                "[devname] decision: stop, written but the read-back failed (not retried): {e}"
            ));
            return Outcome::Unverified {
                backup: backup_path,
                error: e.to_string(),
            };
        }
    };
    let expected = expected_readback(&frames);
    env.log(&format!("[devname] read back 0x51-0x54: {}", hex(&read)));
    if read == expected {
        env.log("[devname] verified: the name read back is the one written");
        Outcome::Verified {
            backup: backup_path,
        }
    } else {
        env.log("[devname] decision: MISMATCH, offer the restore of the backup");
        Outcome::Mismatch {
            read,
            backup: backup_path,
        }
    }
}

/// Prepare without writing (`--dry-run`): validate, build the frames.
///
/// # Errors
///
/// [`NameError`] when `name` is not valid (see [`validate`]).
pub fn prepare(name: &str) -> Result<Vec<Frame>, NameError> {
    frames_for(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const REF_FRAME: [u8; 65] = {
        let mut r = [0u8; 65];
        r[0] = 0x55;
        let n = *b"Alice's keyboard #1";
        let mut i = 0;
        while i < n.len() {
            r[1 + i] = n[i];
            i += 1;
        }
        r
    };

    fn measured_frames() -> Vec<Vec<u8>> {
        vec![
            [&[0x51u8][..], b"Alice's "].concat(),
            [&[0x52u8][..], b"keyboard"].concat(),
            [&[0x53u8][..], b" #1\0\0\0\0\0"].concat(),
            [&[0x54u8][..], &[0u8; 8]].concat(),
        ]
    }

    fn fixture() -> serde_json::Value {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/devname/lion_setdevicename_frames.json");
        serde_json::from_str(
            &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
        )
        .unwrap()
    }

    #[test]
    fn validation() {
        assert_eq!(
            validate("Alice's keyboard #1").unwrap(),
            "Alice's keyboard #1"
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
        assert_eq!(validate("Keyboard ë"), Err(NameError::NotAscii('ë')));
        assert_eq!(validate("a\u{200b}b"), Err(NameError::NotAscii('\u{200b}')));
        assert_eq!(validate("a\\sb"), Err(NameError::Forbidden('\\')));
        for e in [
            NameError::Empty,
            NameError::TooLong(40),
            NameError::TooLongForApple(65),
            NameError::EdgeSpace,
            NameError::Control(1),
            NameError::NotAscii('ë'),
            NameError::Forbidden('\\'),
        ] {
            let got = e.to_string();
            assert!(!got.is_empty(), "{got:?}");
        }
        for b in 0u8..=255 {
            let s = String::from(char::from(b));
            let ok = (0x21..=0x7E).contains(&b) && b != b'\\';
            assert_eq!(validate(&s).is_ok(), ok, "{b:#04x}");
        }
    }

    #[test]
    fn apple_encoding_utf8_truncation_by_whole_characters_and_refusals() {
        assert_eq!(apple_name_bytes("alex").unwrap(), b"alex");
        assert_eq!(apple_name_bytes(&"a".repeat(64)).unwrap().len(), 64);
        assert_eq!(apple_name_bytes(""), Err(NameError::Empty));
        assert_eq!(
            apple_name_bytes(&"a".repeat(65)),
            Err(NameError::TooLongForApple(65))
        );
        assert_eq!(
            apple_name_bytes(&"\u{1F600}".repeat(33)),
            Err(NameError::TooLongForApple(66))
        );
        // 40 × "ë": 40 units (accepted), 80 UTF-8 bytes → trailing characters dropped to 32 × "ë" =
        // 64 bytes, never a split character.
        let e40 = "ë".repeat(40);
        let b = apple_name_bytes(&e40).unwrap();
        assert_eq!(b.len(), 64);
        assert_eq!(std::str::from_utf8(&b).unwrap(), "ë".repeat(32));
        let b = apple_name_bytes(&"\u{1F600}".repeat(32)).unwrap();
        assert_eq!(b.len(), 64);
        assert_eq!(std::str::from_utf8(&b).unwrap(), "\u{1F600}".repeat(16));
        let b = apple_name_bytes(&("a".repeat(63) + "ë")).unwrap();
        assert_eq!(b, "a".repeat(63).into_bytes());
        for n in ["a".repeat(64) + "", "日本語".repeat(21), "ü".repeat(64)] {
            let b = apple_name_bytes(&n).unwrap();
            assert!(b.len() <= 64 && std::str::from_utf8(&b).is_ok(), "{n}");
        }
        let f = long_name_frame(&apple_name_bytes(&e40).unwrap());
        assert_eq!(f.report.len(), 65);
        assert!(f.fields.iter().all(|x| x.end <= 65));
        assert_eq!(frames_for(&e40), Err(NameError::NotAscii('ë')));
        assert_eq!(frames_for(&"a".repeat(33)), Err(NameError::TooLong(33)));
        for n in ["a", "Alice's keyboard #1", &"~".repeat(MAX_NAME_LEN)] {
            let n = validate(n).unwrap();
            assert_eq!(
                apple_name_bytes(&n).unwrap(),
                n.as_bytes(),
                "Apple's model is the identity here"
            );
        }
    }

    #[test]
    fn frames_match_the_lion_fixture_byte_for_byte() {
        let fx = fixture();
        assert_eq!(fx["schema"], 1);
        let r = &fx["report"];
        assert_eq!(r["id"], i64::from(LONG_DEVICE_NAME_ID));
        assert_eq!(r["data_len"], i64::try_from(LONG_DEVICE_NAME_LEN).unwrap());
        assert_eq!(r["userland_len"], 65);
        assert_eq!(r["wire_len"], 66);
        assert_eq!(r["wire_prefix_hex"], "0x53");
        assert_eq!(
            r["max_name_len_utf16"],
            i64::try_from(APPLE_MAX_NAME_UTF16).unwrap()
        );
        assert_eq!(
            r["max_name_bytes_utf8"],
            i64::try_from(LONG_DEVICE_NAME_LEN).unwrap()
        );
        assert_eq!(r["length_prefix"], false);
        assert_eq!(
            fx["sequence"].as_array().unwrap()[0]["wire"],
            "53 55 <64 octets>"
        );
        let never = fx["never_sent_on_this_path"].as_array().unwrap();
        assert!(never
            .iter()
            .any(|v| v.as_str().unwrap().starts_with("0x50")));
        assert!(never
            .iter()
            .any(|v| v.as_str().unwrap().starts_with("0x51-0x54")));
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
            assert_eq!(ex["utf8_len"], i64::try_from(name.len()).unwrap());
            let mut covered = [false; 65];
            for fd in &f.fields {
                assert_eq!(fd.proof, Proof::Disassembly, "{name}: {}", fd.what);
                for c in &mut covered[fd.start..fd.end] {
                    *c = true;
                }
            }
            assert!(covered.iter().all(|&c| c), "{name}");
        }
        let f = &frames_for("Apple A1314 keyboard of owner 01").unwrap()[0];
        assert_eq!(&f.report[33..], &[0u8; 32]);
    }

    #[test]
    fn frame_matches_the_documented_reference_byte_for_byte() {
        let f = frames_for("Alice's keyboard #1").unwrap();
        assert_eq!(f.len(), 1, "one frame: no 0x50, no 0x51-0x54");
        assert_eq!(f[0].report, REF_FRAME.to_vec());
        assert_eq!(f[0].op, WriteOp::DeviceName);
        assert_eq!(f[0].wire()[..3], [0x53, 0x55, b'A']);
        assert_eq!(f[0].wire().len(), 66);
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
        assert!(text.contains("53 55 41 6c 69 63 65 27 73"));
        assert!(text.contains("[disassembly]") && !text.contains("[hypothesis]"));
        let f = frames_for(&"Z".repeat(32)).unwrap();
        assert_eq!(f[0].report.len(), 65);
        assert_eq!(&f[0].report[33..], &[0u8; 32]);
    }

    #[test]
    fn frames_only_use_the_ids_of_the_operation() {
        for name in ["a", "Alice's keyboard #1", &"q".repeat(32)] {
            for f in frames_for(name).unwrap() {
                assert!(f.op.ids().contains(&f.id()));
                assert_eq!(f.data().len(), f.op.payload_len());
                assert!(crate::registry::check_write_op(f.op, f.id(), Direction::Feature).is_ok());
            }
        }
    }

    #[test]
    fn sequence_proof_and_open_points_are_documented() {
        assert!(SEQUENCE_PROOF.contains("0x4d2fe"));
        assert!(RESOLVED_BY_DISASSEMBLY.len() >= 3);
        // U4 and U5 are measured; only U3 stays open.
        for u in ["U4 [measured 2026-10-02]", "U5 [measured 2026-10-02]"] {
            assert!(
                RESOLVED_BY_MEASUREMENT.iter().any(|m| m.starts_with(u)),
                "{u}"
            );
        }
        assert_eq!(UNKNOWNS.len(), 1);
        assert!(UNKNOWNS[0].starts_with("U3 NOT MEASURED"));
        assert!(VALIDATION_EXPERIMENTS
            .iter()
            .any(|e| e.starts_with("E1 DONE")));
    }

    #[test]
    fn readback_and_backup_roundtrip() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        assert_eq!(name_from_raw(&raw).unwrap(), "Alice's keyboard #1");
        let b = Backup::new("AA:BB:CC:DD:EE:F1", &raw, 1_790_000_000, "daemon-cache").unwrap();
        assert_eq!(b.fragments_hex[0], "416c696365277320");
        assert_eq!(b.raw().unwrap(), raw);
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Backup>(&json).unwrap(), b);
        let mut f = measured_frames();
        f[2][0] = 0x52;
        assert!(raw_from_frames(&f).is_none(), "ids out of order");
        assert!(raw_from_frames(&measured_frames()[..3]).is_none());
        let mut bad = b.clone();
        bad.name = Some("autre".into());
        assert!(bad.raw().is_err());
        let mut bad = b.clone();
        bad.name = None;
        assert!(bad.raw().is_err(), "ASCII bytes with no readable copy");
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
        // An empty name is saved (never a reason to refuse a rename), not restored.
        let empty = Backup::new("m", &[0u8; 32], 0, "x").unwrap();
        assert!(empty.raw().is_err());
    }

    #[test]
    fn a_non_ascii_current_name_is_saved_and_restored_byte_for_byte() {
        let mut raw = "Zoë's Keyboard #1".as_bytes().to_vec();
        assert_eq!(raw.len(), 18);
        raw.resize(MAX_NAME_LEN, 0);
        assert_eq!(name_from_raw(&raw), None, "not printable ASCII");
        let b = Backup::new("AA:BB:CC:DD:EE:F1", &raw, 1_790_000_000, "daemon-cache")
            .expect("32 raw bytes are always saveable");
        assert_eq!(b.name, None);
        assert_eq!(b.display_name(), "Zoë's Keyboard #1");
        assert_eq!(b.raw().unwrap(), raw, "the bytes as they are");
        let json = serde_json::to_string(&b).unwrap();
        let back: Backup = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        let f = frames_for_restore(&back).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id(), LONG_DEVICE_NAME_ID);
        assert_eq!(&f[0].data()[..MAX_NAME_LEN], &raw[..]);
        assert!(f[0].data()[MAX_NAME_LEN..].iter().all(|&x| x == 0));
        assert_eq!(expected_readback(&f), raw);
        let odd = Backup::new("m", &[0xffu8; 32], 0, "x").unwrap();
        assert_eq!(odd.raw().unwrap(), vec![0xffu8; 32]);
        assert!(!odd.display_name().chars().any(char::is_control));
        assert!(frames_for("Zoë's Keyboard #1").is_err());
        let mut bad = b.clone();
        bad.name = Some("Keyboard".into());
        assert!(bad.raw().is_err());
    }

    #[test]
    fn a_keyboard_with_a_non_ascii_name_can_be_renamed() {
        let mut raw = "Zoë's Keyboard #1".as_bytes().to_vec();
        raw.resize(MAX_NAME_LEN, 0);
        let mut sim = Sim::new(true);
        sim.cached = Some(raw.clone());
        sim.back = Ok(expected_readback(&frames_for("Desk").unwrap()));
        let o = rename(&mut sim, "Desk");
        assert!(matches!(o, Outcome::Verified { backup: Some(_) }), "{o:?}");
        assert_eq!(sim.backups.len(), 1);
        assert_eq!(sim.backups[0].raw().unwrap(), raw);
        assert_eq!(
            *sim.spy.events.borrow(),
            vec!["backup", "confirm", "write 0x55", "read back"]
        );
    }

    #[test]
    fn the_door_must_be_the_keyboard_of_the_preflight() {
        assert!(check_door_mac("AA:BB:CC:DD:EE:F1", "aa:bb:cc:dd:ee:f1").is_ok());
        let e = check_door_mac("AA:BB:CC:DD:EE:02", "AA:BB:CC:DD:EE:F1").unwrap_err();
        assert!(e.contains("AA:BB:CC:DD:EE:02") && e.contains("AA:BB:CC:DD:EE:F1"));
        // Unknown on either side is never a match.
        assert!(check_door_mac("", "AA:BB:CC:DD:EE:F1").is_err());
        assert!(check_door_mac("AA:BB:CC:DD:EE:F1", "unknown").is_err());
        assert!(check_door_mac("", "").is_err());
    }

    #[test]
    fn read_name_from_reads_the_four_fragments_once_spaced_and_stops_at_the_first_failure() {
        use crate::apple_model::Breaker;
        use crate::decode::{Fixture, HidSource};
        use crate::read_policy::{ConnState, SafeSource, MIN_GAP};
        use std::sync::Mutex;
        struct Counting<'a>(&'a dyn HidSource, RefCell<Vec<u8>>);
        impl HidSource for Counting<'_> {
            fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
                self.1.borrow_mut().push(id);
                self.0.feature(id)
            }
        }
        let mut fx = Fixture::new();
        for f in measured_frames() {
            fx = fx.with(&f);
        }
        let (b, c) = (Mutex::new(Breaker::new()), Mutex::new(ConnState::new()));
        let src = Counting(&fx, RefCell::new(Vec::new()));
        let t0 = std::time::Instant::now();
        let raw = read_name_from(&SafeSource::with_parts(&src, &b, &c)).unwrap();
        assert_eq!(name_from_raw(&raw).unwrap(), "Alice's keyboard #1");
        assert_eq!(raw.len(), MAX_NAME_LEN);
        assert_eq!(
            *src.1.borrow(),
            FRAGMENT_IDS.to_vec(),
            "each id once, in order"
        );
        assert!(t0.elapsed() >= MIN_GAP * 3, "1 s between the requests");
        let fx = Fixture::new()
            .with(&measured_frames()[0])
            .with(&measured_frames()[2]);
        let (b, c) = (Mutex::new(Breaker::new()), Mutex::new(ConnState::new()));
        let src = Counting(&fx, RefCell::new(Vec::new()));
        assert!(read_name_from(&SafeSource::with_parts(&src, &b, &c)).is_err());
        assert_eq!(*src.1.borrow(), vec![0x51, 0x52]);
        let fx = Fixture::new().with(&[0x51, b'a', 0, 0]);
        let e = read_name_from(&fx).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn restore_frames_rewrite_exactly_the_backup() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("m", &raw, 0, "daemon-cache").unwrap();
        let f = frames_for_restore(&b).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(&f[0].data()[..32], &raw[..]);
        assert_eq!(&f[0].data()[32..], &[0u8; 32]);
        assert_eq!(f[0].report, REF_FRAME.to_vec());
    }

    #[test]
    fn stamps_and_private_files() {
        use std::os::unix::fs::PermissionsExt;
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
            control_mtu: Some(672),
        }
    }

    #[test]
    fn preflight_conditions() {
        let got = preflight(&green());
        assert!(got.is_empty(), "{got:?}");
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
                p.battery_state_normal = Some(false);
            },
            PreflightFail::Battery,
        );
        one(
            |p| {
                p.battery_pct = None;
                p.battery_state_normal = None;
            },
            PreflightFail::Battery,
        );
        one(
            |p| {
                p.battery_pct = Some(f64::NAN);
                p.battery_state_normal = None;
            },
            PreflightFail::Battery,
        );
        let mut p = green();
        p.control_mtu = Some(MIN_CONTROL_MTU);
        let got = preflight(&p);
        assert!(got.is_empty(), "{got:?}");
        for m in 0..MIN_CONTROL_MTU {
            p.control_mtu = Some(m);
            assert_eq!(preflight(&p), vec![PreflightFail::ControlMtuTooSmall(m)]);
        }
        assert!(PreflightFail::ControlMtuUnknown.is_mtu());
        assert!(PreflightFail::ControlMtuTooSmall(1).is_mtu());
        assert!(!PreflightFail::Battery.is_mtu());
        let mut p = green();
        p.battery_pct = Some(5.0);
        assert!(preflight(&p).is_empty(), "state normal");
        p.battery_pct = Some(20.0);
        p.battery_state_normal = None;
        assert!(preflight(&p).is_empty(), ">= 20 %");
        assert_eq!(preflight(&Preflight::default()).len(), 4);
        for f in preflight(&Preflight::default()) {
            let got = f.describe();
            assert!(!got.is_empty(), "{got:?}");
        }
    }

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
        connected_at_write: bool,
        calls: u32,
        cached: Option<Vec<u8>>,
        yes: bool,
        backups: Vec<Backup>,
        backup_fails: bool,
        spy: Spy,
        back: Result<Vec<u8>, String>,
        log: Vec<String>,
    }

    impl Sim {
        fn new(yes: bool) -> Self {
            Self {
                pre: green(),
                connected_at_write: true,
                calls: 0,
                cached: raw_from_frames(&measured_frames()),
                yes,
                backups: Vec::new(),
                backup_fails: false,
                spy: Spy::default(),
                back: Err("no answer".into()),
                log: Vec::new(),
            }
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
            "AA:BB:CC:DD:EE:F1".into()
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
        fn confirm(&mut self, _target: &str) -> bool {
            self.spy.events.borrow_mut().push("confirm".into());
            self.yes
        }
        fn sink(&self) -> &dyn FeatureSink {
            &self.spy
        }
        fn read_back(&mut self) -> io::Result<Vec<u8>> {
            self.spy.events.borrow_mut().push("read back".into());
            self.back.clone().map_err(io::Error::other)
        }
        fn log(&mut self, line: &str) {
            self.log.push(line.to_string());
        }
    }

    fn rename(sim: &mut Sim, name: &str) -> Outcome {
        run(&Request::Rename(name.into()), &mut WriteSession::new(), sim)
    }

    #[test]
    fn mtu_unknown_or_too_small_refuses_before_backup_and_confirmation() {
        for (mtu, want) in [
            (None, PreflightFail::ControlMtuUnknown),
            (Some(48), PreflightFail::ControlMtuTooSmall(48)),
            (Some(65), PreflightFail::ControlMtuTooSmall(65)),
        ] {
            let mut sim = Sim::new(true);
            sim.pre.control_mtu = mtu;
            let o = rename(&mut sim, "Desk");
            assert_eq!(o, Outcome::Preflight(vec![want.clone()]), "{mtu:?}");
            assert!(!o.wrote());
            assert_eq!(sim.calls, 1);
            assert!(
                sim.spy.events.borrow().is_empty(),
                "{mtu:?}: no backup, no confirmation, no write"
            );
            assert!(sim.log.iter().any(|l| l.contains("MTU")), "{mtu:?}");
        }
        let mut sim = Sim::new(true);
        sim.pre.control_mtu = Some(MIN_CONTROL_MTU);
        sim.back = Ok(expected_readback(&frames_for("Desk").unwrap()));
        assert!(matches!(rename(&mut sim, "Desk"), Outcome::Verified { .. }));
        assert!(sim
            .log
            .iter()
            .any(|l| l.contains("pre-flight ok") && l.contains("66")));
    }

    #[test]
    fn a_no_writes_nothing_after_the_backup() {
        let mut sim = Sim::new(false);
        let o = rename(&mut sim, "Desk");
        assert_eq!(o, Outcome::Cancelled);
        assert!(!o.wrote());
        assert_eq!(*sim.spy.events.borrow(), vec!["backup", "confirm"]);
        assert!(sim.spy.writes.borrow().is_empty());
        assert!(sim.log.iter().any(|l| l.contains("not confirmed")));
    }

    #[test]
    fn confirmed_the_fixture_frame_is_sent_once_after_the_backup_then_read_back() {
        let fx = fixture();
        for ex in fx["examples"].as_array().unwrap() {
            let name = ex["name"].as_str().unwrap();
            let want = unhex(ex["report_hex"].as_str().unwrap()).unwrap();
            let mut sim = Sim::new(true);
            sim.back = Ok(unhex(ex["readback_0x51_0x54_expected_hex"].as_str().unwrap()).unwrap());
            let o = rename(&mut sim, name);
            assert_eq!(
                o,
                Outcome::Verified {
                    backup: Some("/sim/devname-backup.json".into()),
                },
                "{name}"
            );
            assert_eq!(
                *sim.spy.events.borrow(),
                vec!["backup", "confirm", "write 0x55", "read back"],
                "{name}"
            );
            let w = sim.spy.writes.borrow();
            assert_eq!(w.len(), 1, "{name}: exactly one frame");
            assert_eq!(w[0].0, WriteOp::DeviceName);
            assert_eq!(w[0].1, want, "{name}: the fixture bytes, exactly");
            assert_eq!(w[0].1.len(), 65);
            assert_eq!(sim.backups[0].name.as_deref(), Some("Alice's keyboard #1"));
            assert!(sim.log.iter().any(|l| l.contains(&hex(&w[0].1))));
            assert!(sim
                .log
                .iter()
                .any(|l| l.contains(&format!("wire 53 {}", hex(&w[0].1)))));
            assert_eq!(sim.calls, 2, "pre-flight, then the connection re-check");
        }
    }

    #[test]
    fn reference_frame_reaches_the_spy_byte_for_byte() {
        let mut sim = Sim::new(true);
        sim.back = Ok(raw_from_frames(&measured_frames()).unwrap());
        assert!(matches!(
            rename(&mut sim, "Alice's keyboard #1"),
            Outcome::Verified { .. }
        ));
        assert_eq!(sim.spy.writes.borrow()[0].1, REF_FRAME.to_vec());
    }

    #[test]
    fn refusals_send_nothing() {
        type Case<'a> = (Box<dyn Fn(&mut Sim)>, &'a str, fn(&Outcome) -> bool);
        let cases: Vec<Case> = vec![
            (Box::new(|_| {}), "a\\b", |o| {
                matches!(o, Outcome::InvalidName(_))
            }),
            (Box::new(|s| s.yes = false), "x", |o| {
                matches!(o, Outcome::Cancelled)
            }),
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
            (Box::new(|s| s.pre.control_mtu = None), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (Box::new(|s| s.pre.control_mtu = Some(60)), "x", |o| {
                matches!(o, Outcome::Preflight(_))
            }),
            (
                Box::new(|s| {
                    s.pre.battery_pct = Some(3.0);
                    s.pre.battery_state_normal = Some(false);
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
            let mut sim = Sim::new(true);
            setup(&mut sim);
            let o = rename(&mut sim, name);
            assert!(want(&o), "case {i}: {o:?}");
            assert!(!o.wrote(), "case {i}");
            assert!(
                sim.spy.writes.borrow().is_empty(),
                "case {i}: nothing may be written"
            );
            assert!(
                !sim.spy.events.borrow().iter().any(|e| e == "read back"),
                "case {i}: nothing is read either"
            );
        }
        let mut sim = Sim::new(true);
        sim.backup_fails = true;
        rename(&mut sim, "x");
        assert_eq!(*sim.spy.events.borrow(), vec!["backup"]);
    }

    #[test]
    fn a_used_session_refuses_a_second_write() {
        let mut s = WriteSession::new();
        let mut sim = Sim::new(true);
        sim.back = Ok(expected_readback(&frames_for("x").unwrap()));
        assert!(matches!(
            run(&Request::Rename("x".into()), &mut s, &mut sim),
            Outcome::Verified { .. }
        ));
        let mut sim2 = Sim::new(true);
        let o = run(&Request::Rename("y".into()), &mut s, &mut sim2);
        assert!(
            matches!(o, Outcome::Refused(ref e) if e.contains("already sent")),
            "{o:?}"
        );
        assert!(sim2.spy.writes.borrow().is_empty());
    }

    #[test]
    fn failure_mismatch_and_unreadable_stop_without_retry() {
        let mut sim = Sim::new(true);
        sim.spy.fail = true;
        let o = rename(&mut sim, "x");
        assert!(matches!(o, Outcome::WriteFailed(_)) && o.wrote());
        assert_eq!(sim.spy.writes.borrow().len(), 1, "one attempt only");
        assert!(
            !sim.spy.events.borrow().iter().any(|e| e == "read back"),
            "a failed write is not read back"
        );

        let mut sim = Sim::new(true);
        let o = rename(&mut sim, "x");
        assert!(
            matches!(o, Outcome::Unverified { ref backup, ref error } if backup.is_some() && error.contains("no answer")),
            "{o:?}"
        );
        assert!(o.wrote());
        assert_eq!(sim.spy.writes.borrow().len(), 1);
        assert_eq!(
            sim.spy
                .events
                .borrow()
                .iter()
                .filter(|e| *e == "read back")
                .count(),
            1
        );

        let mut sim = Sim::new(true);
        sim.back = Ok(vec![0; 32]);
        let o = rename(&mut sim, "x");
        assert!(
            matches!(o, Outcome::Mismatch { ref backup, ref read } if backup.is_some() && read == &vec![0; 32])
        );
        assert_eq!(sim.spy.writes.borrow().len(), 1);
    }

    #[test]
    fn restore_saves_the_current_name_then_rewrites_exactly_the_backup() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("AA:BB:CC:DD:EE:F1", &raw, 1, "daemon-cache").unwrap();
        let mut sim = Sim::new(true);
        sim.cached = Some(expected_readback(&frames_for("x").unwrap()));
        sim.back = Ok(raw.clone());
        let o = run(
            &Request::Restore(b.clone()),
            &mut WriteSession::new(),
            &mut sim,
        );
        assert!(matches!(o, Outcome::Verified { backup: Some(_) }));
        assert_eq!(
            *sim.spy.events.borrow(),
            vec!["backup", "confirm", "write 0x55", "read back"]
        );
        assert_eq!(sim.backups[0].mac, "AA:BB:CC:DD:EE:F1");
        {
            let w = sim.spy.writes.borrow();
            assert_eq!(&w[0].1[1..33], &raw[..]);
            assert_eq!(&w[0].1[33..], &[0u8; 32]);
        }
        let mut sim = Sim::new(false);
        assert_eq!(
            run(
                &Request::Restore(b.clone()),
                &mut WriteSession::new(),
                &mut sim
            ),
            Outcome::Cancelled
        );
        assert!(sim.spy.writes.borrow().is_empty());
        let other = Backup::new("AA:BB:CC:DD:EE:02", &raw, 1, "daemon-cache").unwrap();
        let mut sim = Sim::new(true);
        assert_eq!(
            run(&Request::Restore(other), &mut WriteSession::new(), &mut sim),
            Outcome::OtherKeyboard {
                backup: "AA:BB:CC:DD:EE:02".into(),
                target: "AA:BB:CC:DD:EE:F1".into()
            }
        );
        assert!(
            sim.spy.events.borrow().is_empty(),
            "nothing asked, saved or written"
        );
    }
}
