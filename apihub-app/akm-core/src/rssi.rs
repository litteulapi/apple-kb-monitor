//! RSSI and TX power of a connected Bluetooth device.
//!
//! BlueZ MGMT `GET_CONN_INFO` (0x0031) is refused to unprivileged sockets
//! (status 0x14, PERMISSION_DENIED), so the GUI never opens that socket itself.
//! It runs the tiny `rssi-helper` binary (installed by the package with the
//! file capability `cap_net_admin+ep`) as a child process, with a timeout and
//! a 10 s cache (the kernel already caches the answer for 1-3 s).
//!
//! If the helper is missing or not privileged, `read_rssi` returns `None` and
//! `last_error()` gives the reason (logged once per distinct reason).

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Where the package installs the privileged helper.
const HELPER_PATH: &str = "/usr/lib/apple-kb-monitor/rssi-helper";
/// Override for tests / development (not a privilege boundary: the helper
/// itself carries the capability, not this process).
const HELPER_ENV: &str = "APPLE_KB_RSSI_HELPER";
const CACHE_TTL: Duration = Duration::from_secs(10);
const HELPER_TIMEOUT: Duration = Duration::from_millis(1500);
/// BlueZ reports 127 when RSSI / TX power is not available.
const MGMT_VALUE_INVALID: i8 = 127;

/// Why no RSSI could be obtained.
#[derive(Debug, Clone, PartialEq)]
pub enum RssiError {
    BadMac,
    HelperMissing(String),
    HelperSpawn(String),
    Timeout,
    /// Helper exited non-zero; carries its stderr (first line).
    Helper {
        code: Option<i32>,
        msg: String,
    },
    BadOutput(String),
    Unavailable,
}

impl std::fmt::Display for RssiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMac => write!(f, "invalid MAC address"),
            Self::HelperMissing(p) => write!(f, "rssi-helper not installed ({p})"),
            Self::HelperSpawn(e) => write!(f, "cannot run rssi-helper: {e}"),
            Self::Timeout => write!(f, "rssi-helper timed out"),
            Self::Helper { code, msg } => write!(f, "rssi-helper failed (exit {code:?}): {msg}"),
            Self::BadOutput(o) => write!(f, "unparseable rssi-helper output: {o}"),
            Self::Unavailable => write!(f, "RSSI not available for this device"),
        }
    }
}

type Cache = HashMap<String, (Instant, Result<(i8, Option<i8>), RssiError>)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Reason of the last failed `read_rssi`, `None` if the last call succeeded.
#[allow(dead_code)]
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|g| g.clone())
}

/// Read RSSI and TX power (dBm) for `mac` (`"AA:BB:CC:DD:EE:FF"`).
/// Returns `None` on any failure; see `last_error()` for the reason.
pub fn read_rssi(mac: &str) -> Option<(i8, Option<i8>)> {
    let res = read_rssi_cached(mac);
    let mut last = LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner());
    match &res {
        Ok(_) => *last = None,
        Err(e) => {
            let msg = e.to_string();
            if last.as_deref() != Some(msg.as_str()) {
                eprintln!("rssi: {msg}");
            }
            *last = Some(msg);
        }
    }
    res.ok()
}

fn read_rssi_cached(mac: &str) -> Result<(i8, Option<i8>), RssiError> {
    let key = mac.to_ascii_uppercase();
    let now = Instant::now();
    {
        let g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, r)) = g.as_ref().and_then(|c| c.get(&key)) {
            if now.duration_since(*t) < CACHE_TTL {
                return r.clone();
            }
        }
    }
    let path = std::env::var(HELPER_ENV).unwrap_or_else(|_| HELPER_PATH.to_string());
    let res = run_helper(&path, mac, HELPER_TIMEOUT);
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(key, (Instant::now(), res.clone()));
    res
}

/// The helper is `root:akm 0750` (#209): a refusal means the user is not in
/// the `akm` group yet.
fn spawn_error_text(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        "permission denied: add your user to the 'akm' group (sudo usermod -aG akm $USER, then log in again)".to_string()
    } else {
        e.to_string()
    }
}

/// Run the helper once (no cache) and interpret its result.
fn run_helper(path: &str, mac: &str, timeout: Duration) -> Result<(i8, Option<i8>), RssiError> {
    parse_mac(mac).ok_or(RssiError::BadMac)?;
    if !std::path::Path::new(path).exists() {
        return Err(RssiError::HelperMissing(path.to_string()));
    }
    let spawn = || {
        Command::new(path)
            .arg(mac)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    // ETXTBSY: a just-written helper can still be open in a concurrently
    // forked process for a few ms; retry briefly instead of failing.
    let mut child = (0..5)
        .find_map(|i| match spawn() {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && i < 4 => {
                std::thread::sleep(Duration::from_millis(20));
                None
            }
            other => Some(other),
        })
        .expect("last attempt always returns")
        .map_err(|e| RssiError::HelperSpawn(spawn_error_text(&e)))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RssiError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(RssiError::HelperSpawn(e.to_string())),
        }
    };
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.by_ref().take(4096).read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.by_ref().take(4096).read_to_string(&mut err);
    }
    if !status.success() {
        let msg = err.lines().next().unwrap_or("").trim().to_string();
        return Err(RssiError::Helper {
            code: status.code(),
            msg,
        });
    }
    parse_helper_output(&out)
}

/// Parse `{"rssi":-5,"tx_power":4,"max_tx_power":4}`; 127 means unavailable.
fn parse_helper_output(out: &str) -> Result<(i8, Option<i8>), RssiError> {
    let bad = || RssiError::BadOutput(out.trim().chars().take(80).collect());
    let v: serde_json::Value = serde_json::from_str(out.trim()).map_err(|_| bad())?;
    let get = |k: &str| -> Option<i8> { i8::try_from(v.get(k)?.as_i64()?).ok() };
    let rssi = get("rssi").ok_or_else(bad)?;
    if rssi == MGMT_VALUE_INVALID {
        return Err(RssiError::Unavailable);
    }
    let tx = get("tx_power").filter(|t| *t != MGMT_VALUE_INVALID);
    Ok((rssi, tx))
}

/// A timestamped RSSI measurement bound to the MAC it was taken from.
#[derive(Debug, Clone, PartialEq)]
struct Sample {
    mac: String,
    rssi: i32,
    tx: Option<i32>,
    at: Instant,
}

/// Holds the last RSSI measurement and expires it: a value older than
/// `max_age`, taken from another MAC, or superseded by a failed read is
/// reported as absent (never a frozen stale number).
#[derive(Debug, Clone)]
pub struct RssiTracker {
    max_age: Duration,
    sample: Option<Sample>,
}

impl RssiTracker {
    pub fn new(max_age: Duration) -> Self {
        Self {
            max_age,
            sample: None,
        }
    }

    /// Record the outcome of a read for `mac`. `None` (failure) drops any value.
    pub fn record(&mut self, mac: &str, res: Option<(i8, Option<i8>)>, now: Instant) {
        self.sample = res.map(|(r, t)| Sample {
            mac: mac.to_ascii_uppercase(),
            rssi: i32::from(r),
            tx: t.map(i32::from),
            at: now,
        });
    }

    /// Forget everything (device disconnected).
    pub fn clear(&mut self) {
        self.sample = None;
    }

    /// `(rssi_dbm, tx_power_dbm, age)` if a sample for `mac` is still fresh.
    pub fn current(&self, mac: &str, now: Instant) -> Option<(i32, Option<i32>, Duration)> {
        let s = self.sample.as_ref()?;
        let age = now.checked_duration_since(s.at).unwrap_or_default();
        (s.mac.eq_ignore_ascii_case(mac) && age <= self.max_age).then_some((s.rssi, s.tx, age))
    }
}

/// Parse a colon-separated MAC string into 6 bytes.
fn parse_mac(mac: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut octets = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        // Exactly two hex digits: from_str_radix alone accepts "+A" and "1".
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        octets[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_helper(name: &str, body: &str) -> String {
        let dir = std::env::temp_dir().join(format!("rssi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    const MAC: &str = "04:DB:56:CA:42:EE";

    #[test]
    fn parses_valid_output() {
        let o = r#"{"rssi":-42,"tx_power":4,"max_tx_power":4}"#;
        assert_eq!(parse_helper_output(o), Ok((-42, Some(4))));
    }

    #[test]
    fn rejects_invalid_values_and_garbage() {
        assert_eq!(
            parse_helper_output(r#"{"rssi":127,"tx_power":4}"#),
            Err(RssiError::Unavailable)
        );
        assert!(matches!(
            parse_helper_output(""),
            Err(RssiError::BadOutput(_))
        ));
        assert!(matches!(
            parse_helper_output("nope"),
            Err(RssiError::BadOutput(_))
        ));
        assert!(matches!(
            parse_helper_output(r#"{"rssi":null,"tx_power":null}"#),
            Err(RssiError::BadOutput(_))
        ));
        assert!(matches!(
            parse_helper_output(r#"{"rssi":-300,"tx_power":1}"#),
            Err(RssiError::BadOutput(_))
        ));
    }

    #[test]
    fn tx_power_127_does_not_invalidate_rssi() {
        assert_eq!(
            parse_helper_output(r#"{"rssi":-40,"tx_power":127}"#),
            Ok((-40, None))
        );
        assert_eq!(
            parse_helper_output(r#"{"rssi":-40,"tx_power":null}"#),
            Ok((-40, None))
        );
        assert_eq!(parse_helper_output(r#"{"rssi":-40}"#), Ok((-40, None)));
    }

    #[test]
    fn tracker_expires_and_checks_mac() {
        let t0 = Instant::now();
        let mut tr = RssiTracker::new(Duration::from_secs(100));
        assert_eq!(tr.current(MAC, t0), None);
        tr.record(MAC, Some((-50, Some(4))), t0);
        let (r, tx, age) = tr
            .current(&MAC.to_lowercase(), t0 + Duration::from_secs(30))
            .unwrap();
        assert_eq!((r, tx, age), (-50, Some(4), Duration::from_secs(30)));
        assert_eq!(tr.current(MAC, t0 + Duration::from_secs(101)), None);
        assert_eq!(tr.current("AA:BB:CC:DD:EE:FF", t0), None);
        // A failed read drops the value immediately.
        tr.record(MAC, Some((-50, None)), t0);
        tr.record(MAC, None, t0 + Duration::from_secs(1));
        assert_eq!(tr.current(MAC, t0 + Duration::from_secs(2)), None);
        tr.record(MAC, Some((-60, None)), t0);
        tr.clear();
        assert_eq!(tr.current(MAC, t0), None);
    }

    #[test]
    fn missing_helper_is_explicit() {
        let r = run_helper("/nonexistent/rssi-helper", MAC, Duration::from_secs(1));
        assert!(matches!(r, Err(RssiError::HelperMissing(_))));
        assert!(r.unwrap_err().to_string().contains("not installed"));
    }

    #[test]
    fn helper_success() {
        let h = fake_helper("ok", r#"echo '{"rssi":-55,"tx_power":4,"max_tx_power":4}'"#);
        assert_eq!(
            run_helper(&h, MAC, Duration::from_secs(2)),
            Ok((-55, Some(4)))
        );
    }

    #[test]
    fn helper_permission_denied_carries_reason() {
        let h = fake_helper(
            "denied",
            "echo 'rssi-helper: MGMT status 0x14 (permission denied)' >&2; exit 3",
        );
        match run_helper(&h, MAC, Duration::from_secs(2)) {
            Err(RssiError::Helper { code: Some(3), msg }) => assert!(msg.contains("0x14")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn helper_timeout_kills_child() {
        let h = fake_helper("slow", "sleep 5");
        let t = Instant::now();
        assert_eq!(
            run_helper(&h, MAC, Duration::from_millis(200)),
            Err(RssiError::Timeout)
        );
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn bad_mac_never_spawns() {
        assert_eq!(
            run_helper("/bin/true", "not a mac", Duration::from_secs(1)),
            Err(RssiError::BadMac)
        );
        assert_eq!(read_rssi("not a mac"), None);
    }

    #[test]
    fn mac_parsing_is_strict() {
        assert_eq!(
            parse_mac("AA:bb:0C:dd:EE:01"),
            Some([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0x01])
        );
        assert_eq!(parse_mac("+A:bb:0C:dd:EE:01"), None);
        assert_eq!(parse_mac("A:bb:0C:dd:EE:01"), None);
        assert_eq!(parse_mac("AA:bb:0C:dd:EE"), None);
        assert_eq!(parse_mac("AA:bb:0C:dd:EE:01:02"), None);
        assert_eq!(parse_mac("ZZ:bb:0C:dd:EE:01"), None);
        assert_eq!(parse_mac(""), None);
    }
}
