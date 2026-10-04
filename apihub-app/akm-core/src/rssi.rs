//! RSSI and TX power of a connected Bluetooth device.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const HELPER_PATH: &str = "/usr/lib/apple-kb-monitor/rssi-helper";
const CACHE_TTL: Duration = Duration::from_secs(10);
const HELPER_TIMEOUT: Duration = Duration::from_millis(1500);
const MGMT_VALUE_INVALID: i8 = 127;

/// Why no RSSI could be obtained.
#[derive(Debug, Clone, PartialEq)]
pub enum RssiError {
    BadMac,
    HelperMissing(String),
    HelperSpawn(String),
    /// `rssi-helper` exists but is not executable (broken installation).
    Denied,
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
            Self::Denied => write!(f, "cannot run rssi-helper: permission denied"),
            Self::Timeout => write!(f, "rssi-helper timed out"),
            Self::Helper { code, msg } => write!(f, "rssi-helper failed (exit {code:?}): {msg}"),
            Self::BadOutput(o) => write!(f, "unparseable rssi-helper output: {o}"),
            Self::Unavailable => write!(f, "RSSI not available for this device"),
        }
    }
}

type Cache = HashMap<String, (Instant, Result<(i8, Option<i8>), RssiError>)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<RssiError>> = Mutex::new(None);

/// Reason of the last failed `read_rssi`, `None` if the last call succeeded.
pub fn last_error() -> Option<RssiError> {
    LAST_ERROR.lock().ok().and_then(|g| g.clone())
}

/// Why the signal is not measured, as published by the daemon in `radio.rssi_error` of `GetState`:
/// a stable `code` for the interfaces to translate, and the raw `detail`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RssiIssue {
    /// One of the `CODE_*` constants (an unknown code reads as "other").
    pub code: String,
    pub detail: String,
}

/// `rssi-helper` is not installed.
pub const CODE_HELPER_MISSING: &str = "helper_missing";
/// The helper ran but failed (capability lost, Bluetooth controller refused).
pub const CODE_HELPER_FAILED: &str = "helper_failed";
/// The helper did not answer in time.
pub const CODE_TIMEOUT: &str = "timeout";
/// The controller has no value for this link (not connected, out of range).
pub const CODE_UNAVAILABLE: &str = "unavailable";
pub const CODE_OTHER: &str = "other";

/// What to tell the user for an issue code: (reason, fix).
#[must_use]
pub fn explain(code: &str) -> (String, String) {
    use crate::tr;
    match code {
        CODE_HELPER_MISSING => (
            tr!("the rssi-helper utility is not installed"),
            tr!("reinstall the apple-kb-monitor package"),
        ),
        CODE_HELPER_FAILED => (
            tr!("the rssi-helper utility failed (missing capability, Bluetooth refused)"),
            tr!("reinstall the apple-kb-monitor package, then run akmctl doctor"),
        ),
        CODE_TIMEOUT => (
            tr!("the rssi-helper utility does not answer in time"),
            tr!("check the Bluetooth service: systemctl status bluetooth"),
        ),
        CODE_UNAVAILABLE => (
            tr!("the Bluetooth adapter gives no measure for this link"),
            tr!("bring the keyboard closer, or reconnect it"),
        ),
        _ => (
            tr!("the signal cannot be measured"),
            tr!("run akmctl doctor"),
        ),
    }
}

/// One line for a tooltip or a menu: "Signal: not measured (…)".
#[must_use]
pub fn short_line(code: &str) -> String {
    use crate::tr;
    let why = match code {
        CODE_HELPER_MISSING => tr!("rssi-helper not installed"),
        _ => tr!("see akmctl doctor"),
    };
    tr!("Signal: not measured ({why})", why = why)
}

/// `cap_net_admin` in the permitted set of the file capability of `path` (`security.capability`).
#[must_use]
pub fn file_has_cap_net_admin(path: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(path) else {
        return false;
    };
    let mut buf = [0u8; 24];
    // SAFETY: valid NUL-terminated strings, buffer length passed.
    let n = unsafe {
        libc::getxattr(
            c.as_ptr(),
            c"security.capability".as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    cap_data_has_net_admin(&buf[..usize::try_from(n).unwrap_or(0)])
}

/// `vfs_cap_data`: magic/version word, then permitted (low 32 bits) first; `CAP_NET_ADMIN` = 12.
#[must_use]
pub fn cap_data_has_net_admin(d: &[u8]) -> bool {
    d.len() >= 12 && u32::from_le_bytes([d[4], d[5], d[6], d[7]]) & (1 << 12) != 0
}

/// Exit code of `rssi-helper` for a device that is not a connected Apple keyboard.
pub const HELPER_EXIT_NOT_APPLE_KB: i32 = 6;

#[must_use]
pub fn classify(e: &RssiError) -> RssiIssue {
    let code = match e {
        RssiError::HelperMissing(_) => CODE_HELPER_MISSING,
        RssiError::Helper {
            code: Some(HELPER_EXIT_NOT_APPLE_KB),
            ..
        }
        | RssiError::Unavailable => CODE_UNAVAILABLE,
        RssiError::Denied
        | RssiError::Helper { .. }
        | RssiError::HelperSpawn(_)
        | RssiError::BadOutput(_) => CODE_HELPER_FAILED,
        RssiError::Timeout => CODE_TIMEOUT,
        RssiError::BadMac => CODE_OTHER,
    };
    RssiIssue {
        code: code.to_string(),
        detail: e.to_string(),
    }
}

/// Why the last `read_rssi` failed, `None` when it succeeded (or never ran).
#[must_use]
pub fn last_issue() -> Option<RssiIssue> {
    last_error().map(|e| classify(&e))
}

/// Read RSSI and TX power (dBm) for `mac` (`"AA:BB:CC:DD:EE:FF"`).
pub fn read_rssi(mac: &str) -> Option<(i8, Option<i8>)> {
    let res = read_rssi_cached(mac);
    let mut last = LAST_ERROR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match &res {
        Ok(_) => *last = None,
        Err(e) => {
            // Once per distinct cause, at warning priority.
            if last.as_ref() != Some(e) {
                tracing::warn!("rssi: {e}: the signal is not measured");
            }
            *last = Some(e.clone());
        }
    }
    res.ok()
}

fn read_rssi_cached(mac: &str) -> Result<(i8, Option<i8>), RssiError> {
    let key = mac.to_ascii_uppercase();
    let now = Instant::now();
    {
        let g = CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((t, r)) = g.as_ref().and_then(|c| c.get(&key)) {
            if now.duration_since(*t) < CACHE_TTL {
                return r.clone();
            }
        }
    }
    let hci = crate::hidraw::hci_index_for_mac_in(std::path::Path::new("/sys"), mac);
    let res = run_helper(HELPER_PATH, mac, hci, HELPER_TIMEOUT);
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(key, (Instant::now(), res.clone()));
    res
}

fn spawn_error(e: &std::io::Error) -> RssiError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        RssiError::Denied
    } else {
        RssiError::HelperSpawn(e.to_string())
    }
}

/// Block until `child` exits or `timeout` passes (pidfd); returns at once without pidfd support,
/// the caller's `try_wait` loop then does the waiting.
fn wait_exit(child: &std::process::Child, timeout: Duration) {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: pidfd_open(2) on the pid of our own, not yet reaped child.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id(), 0) };
    let Ok(raw) = i32::try_from(raw) else { return };
    if raw < 0 {
        return;
    }
    // SAFETY: `raw` is a fresh descriptor owned by nobody else.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut p = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = libc::c_int::try_from(timeout.as_millis()).unwrap_or(libc::c_int::MAX);
    // SAFETY: one valid pollfd; an EINTR simply falls back to the loop.
    unsafe { libc::poll(&raw mut p, 1, ms) };
}

fn run_helper(
    path: &str,
    mac: &str,
    hci: Option<u16>,
    timeout: Duration,
) -> Result<(i8, Option<i8>), RssiError> {
    parse_mac(mac).ok_or(RssiError::BadMac)?;
    if !std::path::Path::new(path).exists() {
        return Err(RssiError::HelperMissing(path.to_string()));
    }
    let spawn = || {
        Command::new(path)
            .arg(mac)
            .args(hci.map(|i| i.to_string()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    // ETXTBSY: a just-written helper can still be open in a concurrently forked process for a few
    // ms; retry briefly instead of failing.
    let mut retries = 0;
    let mut child = loop {
        match spawn() {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && retries < 4 => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            other => break other.map_err(|e| spawn_error(&e))?,
        }
    };
    let deadline = Instant::now() + timeout;
    wait_exit(&child, timeout);
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

#[derive(Debug, Clone, PartialEq)]
struct Sample {
    mac: String,
    rssi: i32,
    tx: Option<i32>,
    at: Instant,
}

/// Holds the last RSSI measurement and expires it.
#[derive(Debug, Clone)]
pub struct RssiTracker {
    max_age: Duration,
    sample: Option<Sample>,
}

impl RssiTracker {
    #[must_use]
    pub fn new(max_age: Duration) -> Self {
        Self {
            max_age,
            sample: None,
        }
    }

    /// Record the outcome of a read for `mac`.
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
    #[must_use]
    pub fn current(&self, mac: &str, now: Instant) -> Option<(i32, Option<i32>, Duration)> {
        let s = self.sample.as_ref()?;
        let age = now.checked_duration_since(s.at).unwrap_or_default();
        (s.mac.eq_ignore_ascii_case(mac) && age <= self.max_age).then_some((s.rssi, s.tx, age))
    }
}

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

    #[test]
    fn capability_bits() {
        let mut d = [0u8; 20];
        d[0..4].copy_from_slice(&0x0200_0001u32.to_le_bytes());
        assert!(!cap_data_has_net_admin(&d));
        d[4..8].copy_from_slice(&(1u32 << 12).to_le_bytes());
        assert!(cap_data_has_net_admin(&d));
        assert!(!cap_data_has_net_admin(&d[..8]));
        assert!(!file_has_cap_net_admin("/nonexistent/rssi-helper"));
    }

    #[test]
    fn denied_detail_has_no_group_recipe() {
        let t = RssiError::Denied.to_string();
        assert!(
            !t.contains("akm") && !t.contains("usermod") && !t.contains("restart"),
            "{t}"
        );
    }
    use std::os::unix::fs::PermissionsExt;

    /// A shell script standing in for the helper, removed with its directory on drop.
    struct FakeHelper {
        dir: std::path::PathBuf,
        path: String,
    }

    impl std::ops::Deref for FakeHelper {
        type Target = str;
        fn deref(&self) -> &str {
            &self.path
        }
    }

    impl Drop for FakeHelper {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fake_helper(name: &str, body: &str) -> FakeHelper {
        let dir = std::env::temp_dir().join(format!("rssi-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeHelper {
            dir,
            path: p.to_string_lossy().into_owned(),
        }
    }

    const MAC: &str = "AA:BB:CC:DD:EE:F1";

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
        tr.record(MAC, Some((-50, None)), t0);
        tr.record(MAC, None, t0 + Duration::from_secs(1));
        assert_eq!(tr.current(MAC, t0 + Duration::from_secs(2)), None);
        tr.record(MAC, Some((-60, None)), t0);
        tr.clear();
        assert_eq!(tr.current(MAC, t0), None);
    }

    #[test]
    fn missing_helper_is_explicit() {
        let r = run_helper(
            "/nonexistent/rssi-helper",
            MAC,
            None,
            Duration::from_secs(1),
        );
        assert!(matches!(r, Err(RssiError::HelperMissing(_))));
        assert!(r.unwrap_err().to_string().contains("not installed"));
    }

    #[test]
    fn helper_success() {
        let h = fake_helper("ok", r#"echo '{"rssi":-55,"tx_power":4,"max_tx_power":4}'"#);
        assert_eq!(
            run_helper(&h, MAC, None, Duration::from_secs(2)),
            Ok((-55, Some(4)))
        );
    }

    #[test]
    fn helper_gets_the_controller_index() {
        let h = fake_helper(
            "hci",
            r#"[ "$#" = 2 ] && [ "$2" = 1 ] || exit 3; echo '{"rssi":-41,"tx_power":4,"max_tx_power":4}'"#,
        );
        assert_eq!(
            run_helper(&h, MAC, Some(1), Duration::from_secs(2)),
            Ok((-41, Some(4)))
        );
        assert!(run_helper(&h, MAC, None, Duration::from_secs(2)).is_err());
    }

    #[test]
    fn helper_permission_denied_carries_reason() {
        let h = fake_helper(
            "denied",
            "echo 'rssi-helper: MGMT status 0x14 (permission denied)' >&2; exit 3",
        );
        match run_helper(&h, MAC, None, Duration::from_secs(2)) {
            Err(RssiError::Helper { code: Some(3), msg }) => assert!(msg.contains("0x14")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_exit_wait_ends_on_exit_or_timeout() {
        let mut quick = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let t = Instant::now();
        wait_exit(&quick, Duration::from_secs(5));
        assert!(t.elapsed() < Duration::from_secs(2), "woken by the exit");
        quick.wait().unwrap();
        let mut slow = Command::new("sleep").arg("5").spawn().unwrap();
        let t = Instant::now();
        wait_exit(&slow, Duration::from_millis(100));
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "bounded by the timeout"
        );
        slow.kill().unwrap();
        slow.wait().unwrap();
    }

    #[test]
    fn helper_timeout_kills_child() {
        let h = fake_helper("slow", "sleep 5");
        let t = Instant::now();
        assert_eq!(
            run_helper(&h, MAC, None, Duration::from_millis(200)),
            Err(RssiError::Timeout)
        );
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn bad_mac_never_spawns() {
        assert_eq!(
            run_helper("/bin/true", "not a mac", None, Duration::from_secs(1)),
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

    #[test]
    fn helper_not_executable_is_a_refusal_with_a_code() {
        let p = fake_helper("noexec", "exit 0");
        std::fs::set_permissions(&*p, std::fs::Permissions::from_mode(0o644)).unwrap();
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let e = run_helper(&p, MAC, None, Duration::from_secs(2)).unwrap_err();
        assert_eq!(e, RssiError::Denied);
        assert_eq!(classify(&e).code, CODE_HELPER_FAILED);
    }

    #[test]
    fn every_error_has_a_stable_code() {
        let c = |e: RssiError| classify(&e).code;
        assert_eq!(
            c(RssiError::HelperMissing("/x".into())),
            CODE_HELPER_MISSING
        );
        assert_eq!(c(RssiError::Timeout), CODE_TIMEOUT);
        assert_eq!(c(RssiError::Unavailable), CODE_UNAVAILABLE);
        assert_eq!(
            c(RssiError::Helper {
                code: Some(3),
                msg: "MGMT status 0x14".into()
            }),
            CODE_HELPER_FAILED
        );
        assert_eq!(c(RssiError::BadMac), CODE_OTHER);
        assert_eq!(
            c(RssiError::Helper {
                code: Some(6),
                msg: "refused".into()
            }),
            CODE_UNAVAILABLE
        );
    }

    #[test]
    fn issue_json_is_code_and_detail() {
        let i = classify(&RssiError::Timeout);
        let j = serde_json::to_value(&i).unwrap();
        assert_eq!(j["code"], "timeout");
        assert_eq!(j["detail"], "rssi-helper timed out");
        let back: RssiIssue = serde_json::from_str(r#"{"code":"x"}"#).unwrap();
        assert_eq!(back.detail, "");
    }

    #[test]
    fn every_code_is_explained_with_an_action() {
        for c in [
            CODE_HELPER_MISSING,
            CODE_HELPER_FAILED,
            CODE_TIMEOUT,
            CODE_UNAVAILABLE,
            CODE_OTHER,
        ] {
            let (why, fix) = explain(c);
            assert!(!why.is_empty() && !fix.is_empty(), "{c}");
        }
        assert_ne!(explain(CODE_HELPER_FAILED), explain(CODE_OTHER));
    }
}
