//! The name stored **in** the keyboard ("nom propre", #248, #192), as
//! opposed to the alias of this computer (`akmctl rename <nom>`, BlueZ
//! `Alias`, [`crate::alias`]), which stays the default way to rename.
//!
//! What Apple does (docs/RENOMMER-CLAVIER.md §5, RE-PILOTES-ANCIENS.md §5 L11):
//! Lion 10.7 `-[AppleBluetoothHIDDevice setDeviceName:]`, for a PID whose
//! personality declares `LongDeviceName` (the 598 = `0x0256` does), sends
//! **one** SET Feature `0x55` of 64 data bytes, then an HCI remote name
//! request; the 4-fragment path (`0x51`-`0x54` then `0x50` `DeviceNameChange`)
//! is only for PIDs without `LongDeviceName`. macOS 26.5 no longer writes the
//! name at all (RE-MACOS-SILICON.md §3.4). The name is read back with
//! `deviceNameFromHardware` = GET `0x51`-`0x54` (4 × 8 ASCII bytes).
//!
//! What is **not** proven: the bytes of the 64-byte name field (encoding,
//! terminator, padding), whether the write persists, whether `0x51`-`0x54`
//! reflect it, the HIDP MTU of this keyboard ([`UNKNOWNS`]). Hence
//! [`SEQUENCE_PROOF`] is [`SequenceProof::NotProven`] and [`run`] refuses the
//! real write before anything is touched; `--dry-run` prepares everything
//! (validation, pre-flight, backup, frames with the proof level of every
//! byte). The hardware door ([`crate::hidraw::hid_write_feature`]) has no
//! 65-byte ioctl either: a second, independent barrier.
//!
//! Everything here is pure or goes through [`RenameEnv`]: the tests use a
//! simulated environment and a spy [`FeatureSink`], never the hardware.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::parity::FeatureSink;
use crate::registry::{Direction, Proof, WriteOp, WriteSession};

/// `LongDeviceName` report id [plist] [désassemblage].
pub const LONG_DEVICE_NAME_ID: u8 = 0x55;
/// Data bytes of `LongDeviceName` [plist] (`size` = 64; `getMaxDeviceNameLength` = 64).
pub const LONG_DEVICE_NAME_LEN: usize = 64;
/// `DeviceName1..4`, read back after a reconnection [plist] [mesuré].
pub const FRAGMENT_IDS: [u8; 4] = [0x51, 0x52, 0x53, 0x54];
/// Data bytes of each fragment [plist] [mesuré].
pub const FRAGMENT_LEN: usize = 8;
/// Longest name accepted here: what `0x51`-`0x54` can show back (4 × 8), so
/// that every write can be verified. Apple's UI allows 64.
pub const MAX_NAME_LEN: usize = FRAGMENT_IDS.len() * FRAGMENT_LEN;
/// Timeout Apple passes to `IOHIDDeviceInterface::setReport` (1000 ms)
/// [désassemblage]: a bound on the HANDSHAKE wait, not a delay between frames.
pub const APPLE_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(1000);
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

/// Is the exact byte sequence Apple sends established, byte for byte, by a
/// reference frame of the documentation? Only [`SequenceProof::Proven`] lets
/// [`run`] write; production code passes [`SEQUENCE_PROOF`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceProof {
    NotProven,
    Proven,
}

/// State of the proof today: see [`UNKNOWNS`] and docs/RENOMMER-CLAVIER.md §5.
pub const SEQUENCE_PROOF: SequenceProof = SequenceProof::NotProven;

/// What the documents do not establish (docs/RENOMMER-CLAVIER.md §5.3).
pub const UNKNOWNS: &[&str] = &[
    "U1 content of the 64 name bytes of 0x55: encoding (ASCII / UTF-8 / MacRoman), NUL terminator, padding (NUL? space?), length prefix: L11 gives the id and the size only, no listing nor address",
    "U2 RE-MACOS-SILICON §3.4 calls the write order a deduction ('non observé chez Apple') while RE-PILOTES-ANCIENS L11 cites a Lion disassembly: the Lion listing of setDeviceName: is not in the repository",
    "U3 persistence: NVRAM or volatile (the Magic Keyboard descriptor declares its 0x55 Feature 'volatile', RE-COMMANDES-VENDEUR §1.1); lost at a battery change?",
    "U4 do 0x51-0x54 reflect a 0x55 write (first 32 bytes?), and when (at once, after a reset, after a reconnection)?",
    "U5 does the firmware 0x0050 answer HANDSHAKE SUCCESSFUL to SET 0x55 (the GET is refused 0x03, which says nothing about SET)?",
    "U6 Linux path: hidraw set-feature ioctl of 65 bytes -> hidp -> 66 bytes on the HIDP control channel: negotiated L2CAP MTU of this keyboard unknown (Apple fragments with DATC, RE-GHIDRA-KEXT §2.5)",
    "U7 role of 0x50 DeviceNameChange: only on the 4-fragment path per L11 (never sent here); never written by any macOS examined",
];

/// Passive experiments that would lift the unknowns (no write to the keyboard).
pub const VALIDATION_EXPERIMENTS: &[&str] = &[
    "E1 (U1, U2, U7) Ghidra on Lion 10.7.5 IOBluetooth.framework: -[AppleBluetoothHIDDevice setDeviceName:] address, string conversion (getCString:maxLength:encoding: and its encoding constant), buffer size handed to setReport, memset / padding, timeout, 0x50 or not; commit the reference frame of 'Clavier de maria #1' in docs/RENOMMER-CLAVIER.md §5.2",
    "E2 (U6) btmon capture of an ordinary reconnection of the keyboard: L2CAP Configure Request / Response MTU of PSM 17 (control); passive",
    "E3 (U3) compare the cached 0x51-0x54, HID_NAME and BlueZ Name before and after a battery change (daemon cache, no new read)",
    "E4 (U3, U5) public HID descriptors of Apple keyboards of the same generation declaring 0x55: Feature flags (volatile / non-volatile) and size",
];

// ── validation ─────────────────────────────────────────────────────────────

/// Why a name is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong(usize),
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
    /// added by the Bluetooth stack, then the report [désassemblage] (setReportWL).
    pub fn wire(&self) -> Vec<u8> {
        let mut w = vec![0x53];
        w.extend_from_slice(&self.report);
        w
    }
}

/// The 64 data bytes of `0x55` for the 32 bytes `raw` of a name: `raw`, then
/// NUL up to 64 (hypothesis U1).
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
            what: "report id 0x55 LongDeviceName: the only report Lion's setDeviceName: writes for a PID that declares it (L11) [plist + désassemblage]",
        },
        Field {
            start: 1,
            end: 1 + LONG_DEVICE_NAME_LEN,
            proof: Proof::Plist,
            what: "64 data bytes: size of LongDeviceName in the 598 personality, getMaxDeviceNameLength = 64 [plist + désassemblage]",
        },
    ];
    if n > 0 {
        fields.push(Field {
            start: 1,
            end: 1 + n,
            proof: Proof::Hypothesis,
            what:
                "name as ASCII bytes, from byte 1 (U1; the read-back fragments are ASCII [mesuré])",
        });
    }
    fields.push(Field {
        start: 1 + n,
        end: 1 + LONG_DEVICE_NAME_LEN,
        proof: Proof::Hypothesis,
        what: "NUL padding to 64 bytes (U1; 0x53 / 0x54 read back NUL-padded [mesuré])",
    });
    Frame {
        op: WriteOp::DeviceName,
        report,
        fields,
    }
}

/// The frames of Apple's rename for a validated `name`: exactly one SET
/// Feature `0x55` (no `0x50`, no `0x51`-`0x54`: L11 for a PID with
/// `LongDeviceName`).
pub fn frames_for(name: &str) -> Result<Vec<Frame>, NameError> {
    let name = validate(name)?;
    Ok(vec![long_name_frame(name.as_bytes())])
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
        "then: no other frame (no 0x50, no 0x51-0x54); HANDSHAKE awaited at most {} ms [désassemblage]; \
         the name is read back (0x51-0x54) only after a reconnection\n",
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

/// Facts gathered before a write (from the daemon and `akmctl doctor`).
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
}

impl PreflightFail {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::NotConnected => "keyboard not connected",
            Self::Battery => "battery below 20 % and state not 'normal' (or unknown)",
            Self::BreakerOpen => "circuit breaker open: the keyboard stopped answering",
            Self::RecentReadFailure => "the last hardware read failed or was incomplete",
            Self::DoctorNotGreen => {
                "`akmctl doctor` is not green (link health, pairing, configuration)"
            }
            Self::NotInteractive => {
                "not an interactive terminal (stdin and stdout must be a terminal)"
            }
        }
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
    v
}

// ── orchestration ──────────────────────────────────────────────────────────

/// Outcome of waiting for the keyboard after the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconnect {
    /// A new connection, with the 32 bytes `0x51`-`0x54` read in it.
    Back(Vec<u8>),
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
    /// Pre-flight failed: nothing was touched.
    Preflight(Vec<PreflightFail>),
    /// The daemon has no complete `0x51`-`0x54` for this connection.
    NoCachedName,
    /// The backup could not be written: nothing was written to the keyboard.
    BackupFailed(String),
    /// The typed confirmation did not match: nothing was written.
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
    Verified { backup: Option<PathBuf> },
    /// Written, read back different: offer the guided restore.
    Mismatch {
        read: Vec<u8>,
        backup: Option<PathBuf>,
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

/// The guarded rename (or restore). Order: name, proof, pre-flight, cached
/// bytes, BACKUP (rename only), typed confirmation, connection re-checked,
/// ONE write per frame through `session` (each id once), wait for the
/// reconnection, read back, compare. Stops at the first failure, never
/// retries, logs every byte and every decision.
pub fn run(
    req: &Request,
    proof: SequenceProof,
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
    if proof != SequenceProof::Proven {
        env.log("[devname] decision: real write REFUSED (NotProven): the bytes Apple sends are not established byte for byte (docs/RENOMMER-CLAVIER.md §5.3); nothing was touched");
        return Outcome::NotProven;
    }
    let fails = preflight(&env.preflight());
    if !fails.is_empty() {
        for f in &fails {
            env.log(&format!("[devname] pre-flight failed: {}", f.describe()));
        }
        return Outcome::Preflight(fails);
    }
    env.log("[devname] pre-flight ok");
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
        "This writes {} frame(s) into the keyboard's firmware ({}). Type the name exactly ({target:?}) then Enter, anything else cancels: ",
        frames.len(),
        frames.iter().map(|f| format!("{:#04x}", f.id())).collect::<Vec<_>>().join(", ")
    );
    if !env.confirm(&prompt, &target) {
        env.log(
            "[devname] decision: cancelled (typed confirmation does not match); nothing written",
        );
        return Outcome::Cancelled;
    }
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
    let read = match env.wait_reconnect(RECONNECT_WAIT) {
        Reconnect::Back(r) => r,
        Reconnect::Timeout => {
            env.log(
                "[devname] decision: stop, the keyboard did not come back in time; no new attempt",
            );
            return Outcome::NoReconnect;
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
        env.log("[devname] decision: MISMATCH, offer the guided restore of the backup");
        Outcome::Mismatch {
            read,
            backup: backup_path,
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

    /// Reference of docs/RENOMMER-CLAVIER.md §5.2 (hypothesis U1): the report
    /// for the measured name "Clavier de maria #1".
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
            f[0].fields.iter().any(|x| x.proof == Proof::Hypothesis),
            "the name bytes are a hypothesis"
        );
        // The read-back of the reference is the measured fragments.
        assert_eq!(
            expected_readback(&f),
            raw_from_frames(&measured_frames()).unwrap()
        );
        let text = render_frames(&f);
        assert!(text.contains("53 55 43 6c 61 76 69 65 72"));
        assert!(
            text.contains("[hypothèse]")
                && text.contains("[plist]")
                && text.contains("[désassemblage]")
        );
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
    fn production_state_is_not_proven() {
        assert_eq!(SEQUENCE_PROOF, SequenceProof::NotProven);
        assert!(UNKNOWNS.len() >= 5 && VALIDATION_EXPERIMENTS.len() >= 3);
        // No production source constructs `SequenceProof::Proven`: only the
        // tests of this module may (the flag flips with a reviewed commit).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for d in [
            "akm-core/src",
            "apple-kb-monitord/src",
            "crates/akmctl/src",
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
                    let prod = if p.ends_with("akm-core/src/devname.rs") {
                        text.split("#[cfg(test)]").next().unwrap().to_string()
                    } else {
                        text
                    };
                    let hits = prod.matches("SequenceProof::Proven").count();
                    let allowed = usize::from(p.ends_with("akm-core/src/devname.rs")) * 2;
                    assert!(
                        hits <= allowed,
                        "{} names SequenceProof::Proven",
                        p.display()
                    );
                }
            }
        }
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
        // Either condition is enough.
        let mut p = green();
        p.battery_pct = Some(5.0);
        assert!(preflight(&p).is_empty(), "state normal");
        p.battery_pct = Some(20.0);
        p.battery_state_normal = None;
        assert!(preflight(&p).is_empty(), ">= 20 %");
        assert_eq!(preflight(&Preflight::default()).len(), 4);
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

    fn rename(sim: &mut Sim, name: &str, proof: SequenceProof) -> Outcome {
        run(
            &Request::Rename(name.into()),
            proof,
            &mut WriteSession::new(),
            sim,
        )
    }

    #[test]
    fn production_proof_refuses_before_touching_anything() {
        let mut sim = Sim::new("Bureau");
        let o = rename(&mut sim, "Bureau", SEQUENCE_PROOF);
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
    fn full_sequence_on_a_spy_backup_first_then_one_write_then_verify() {
        let mut sim = Sim::new("Bureau");
        let expected = expected_readback(&frames_for("Bureau").unwrap());
        sim.back = Reconnect::Back(expected);
        let o = rename(&mut sim, "Bureau", SequenceProof::Proven);
        assert_eq!(
            o,
            Outcome::Verified {
                backup: Some("/sim/devname-backup.json".into())
            }
        );
        // Order: backup, confirmation, then the single write.
        assert_eq!(
            *sim.spy.events.borrow(),
            vec!["backup", "confirm", "write 0x55"]
        );
        let w = sim.spy.writes.borrow();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].0, WriteOp::DeviceName);
        assert_eq!(w[0].1, frames_for("Bureau").unwrap()[0].report);
        assert_eq!(sim.backups[0].name, "Clavier de maria #1");
        // Every byte sent is in the journal.
        assert!(sim.log.iter().any(|l| l.contains(&hex(&w[0].1))));
    }

    #[test]
    fn reference_frame_reaches_the_spy_byte_for_byte() {
        let mut sim = Sim::new("Clavier de maria #1");
        sim.back = Reconnect::Back(raw_from_frames(&measured_frames()).unwrap());
        assert!(matches!(
            rename(&mut sim, "Clavier de maria #1", SequenceProof::Proven),
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
            let o = rename(&mut sim, name, SequenceProof::Proven);
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
        rename(&mut sim, "x", SequenceProof::Proven);
        assert_eq!(*sim.spy.events.borrow(), vec!["backup"]);
    }

    #[test]
    fn a_used_session_refuses_a_second_write() {
        let mut s = WriteSession::new();
        let mut sim = Sim::new("x");
        sim.back = Reconnect::Back(expected_readback(&frames_for("x").unwrap()));
        assert!(matches!(
            run(
                &Request::Rename("x".into()),
                SequenceProof::Proven,
                &mut s,
                &mut sim
            ),
            Outcome::Verified { .. }
        ));
        let mut sim2 = Sim::new("y");
        let o = run(
            &Request::Rename("y".into()),
            SequenceProof::Proven,
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
        let o = rename(&mut sim, "x", SequenceProof::Proven);
        assert!(matches!(o, Outcome::WriteFailed(_)) && o.wrote());
        assert_eq!(sim.spy.writes.borrow().len(), 1, "one attempt only");

        let mut sim = Sim::new("x");
        let o = rename(&mut sim, "x", SequenceProof::Proven);
        assert_eq!(o, Outcome::NoReconnect);
        assert_eq!(sim.spy.writes.borrow().len(), 1);

        let mut sim = Sim::new("x");
        sim.back = Reconnect::Back(vec![0; 32]);
        let o = rename(&mut sim, "x", SequenceProof::Proven);
        assert!(matches!(o, Outcome::Mismatch { ref backup, .. } if backup.is_some()));
        assert_eq!(sim.spy.writes.borrow().len(), 1);
    }

    #[test]
    fn restore_rewrites_exactly_the_backup_with_a_new_confirmation_and_no_new_backup() {
        let raw = raw_from_frames(&measured_frames()).unwrap();
        let b = Backup::new("04:DB:56:CA:42:EE", &raw, 1, "daemon-cache").unwrap();
        let mut sim = Sim::new("Clavier de maria #1");
        sim.cached = Some(expected_readback(&frames_for("x").unwrap()));
        sim.back = Reconnect::Back(raw.clone());
        let o = run(
            &Request::Restore(b.clone()),
            SequenceProof::Proven,
            &mut WriteSession::new(),
            &mut sim,
        );
        assert_eq!(o, Outcome::Verified { backup: None });
        assert_eq!(*sim.spy.events.borrow(), vec!["confirm", "write 0x55"]);
        let w = sim.spy.writes.borrow();
        assert_eq!(&w[0].1[1..33], &raw[..]);
        assert_eq!(&w[0].1[33..], &[0u8; 32]);
        // Not confirmed: nothing.
        let mut sim = Sim::new("no");
        assert_eq!(
            run(
                &Request::Restore(b),
                SequenceProof::Proven,
                &mut WriteSession::new(),
                &mut sim
            ),
            Outcome::Cancelled
        );
        assert!(sim.spy.writes.borrow().is_empty());
        // Restore is refused as well while the sequence is not proven.
        let mut sim = Sim::new("x");
        let b = Backup::new("m", &raw, 1, "daemon-cache").unwrap();
        assert_eq!(
            run(
                &Request::Restore(b),
                SEQUENCE_PROOF,
                &mut WriteSession::new(),
                &mut sim
            ),
            Outcome::NotProven
        );
    }
}
