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
    /// `rssi-helper` exists but this process may not run it (`root:akm
    /// 0750`, #209): not in the group `akm`, or not yet in this session.
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
            Self::Denied => write!(f, "cannot run rssi-helper: {}", DENIED_TEXT),
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

/// Why the signal is not measured, as published by the daemon in
/// `radio.rssi_error` of `GetState` (#269): a stable `code` for the
/// interfaces to translate, and the raw `detail` (English, for logs).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RssiIssue {
    /// One of the `CODE_*` constants (an unknown code reads as "other").
    pub code: String,
    pub detail: String,
}

/// Not a member of the group `akm`: `sudo usermod -aG akm $USER`.
pub const CODE_NOT_IN_GROUP: &str = "not_in_group";
/// Listed in `akm`, but the process predates it: a new session is needed
/// (with lingering, the whole user manager: reboot, or log out everywhere).
pub const CODE_NEEDS_RELOGIN: &str = "needs_relogin";
/// `rssi-helper` is not installed.
pub const CODE_HELPER_MISSING: &str = "helper_missing";
/// The helper ran but failed (capability lost, Bluetooth controller refused).
pub const CODE_HELPER_FAILED: &str = "helper_failed";
/// The helper did not answer in time.
pub const CODE_TIMEOUT: &str = "timeout";
/// The controller has no value for this link (not connected, out of range).
pub const CODE_UNAVAILABLE: &str = "unavailable";
pub const CODE_OTHER: &str = "other";

const DENIED_TEXT: &str =
    "permission denied: add your user to the 'akm' group (sudo usermod -aG akm $USER, then start a new session)";

/// What to tell the user for an issue code: (reason, fix), in French or
/// English. One text for every interface (window, tray, Diagnose, akmctl).
/// The group is read by a process only when it starts: the daemon runs in
/// the user manager, which outlives a logout when lingering is enabled, so
/// the fix says "restart the computer", never only "log in again".
pub fn explain(code: &str, french: bool) -> (&'static str, &'static str) {
    match (code, french) {
        (CODE_NOT_IN_GROUP, false) => (
            "your account is not in the group akm: the signal cannot be measured",
            "sudo usermod -aG akm $USER, then restart the computer (logging out is not enough)",
        ),
        (CODE_NOT_IN_GROUP, true) => (
            "votre compte n'est pas dans le groupe akm : le signal ne peut pas être mesuré",
            "sudo usermod -aG akm $USER, puis redémarrez l'ordinateur (se déconnecter ne suffit pas)",
        ),
        (CODE_NEEDS_RELOGIN, false) => (
            "you are in the group akm, but the service started before and does not see it yet",
            "restart the computer (logging out is not enough: the service keeps its old groups)",
        ),
        (CODE_NEEDS_RELOGIN, true) => (
            "vous êtes dans le groupe akm, mais le service a démarré avant et ne le voit pas encore",
            "redémarrez l'ordinateur (se déconnecter ne suffit pas : le service garde ses anciens groupes)",
        ),
        (CODE_HELPER_MISSING, false) => (
            "the rssi-helper utility is not installed",
            "reinstall the apple-kb-monitor package",
        ),
        (CODE_HELPER_MISSING, true) => (
            "l'utilitaire rssi-helper n'est pas installé",
            "réinstallez le paquet apple-kb-monitor",
        ),
        (CODE_HELPER_FAILED, false) => (
            "the rssi-helper utility failed (missing capability, Bluetooth refused)",
            "reinstall the apple-kb-monitor package, then run akmctl doctor",
        ),
        (CODE_HELPER_FAILED, true) => (
            "l'utilitaire rssi-helper a échoué (capacité absente, refus du Bluetooth)",
            "réinstallez le paquet apple-kb-monitor, puis lancez akmctl doctor",
        ),
        (CODE_TIMEOUT, false) => (
            "the rssi-helper utility does not answer in time",
            "check the Bluetooth service: systemctl status bluetooth",
        ),
        (CODE_TIMEOUT, true) => (
            "l'utilitaire rssi-helper ne répond pas à temps",
            "vérifiez le service Bluetooth : systemctl status bluetooth",
        ),
        (CODE_UNAVAILABLE, false) => (
            "the Bluetooth adapter gives no measure for this link",
            "bring the keyboard closer, or reconnect it",
        ),
        (CODE_UNAVAILABLE, true) => (
            "l'adaptateur Bluetooth ne donne aucune mesure pour cette liaison",
            "rapprochez le clavier, ou reconnectez-le",
        ),
        (_, false) => ("the signal cannot be measured", "run akmctl doctor"),
        (_, true) => ("le signal ne peut pas être mesuré", "lancez akmctl doctor"),
    }
}

/// One line for a tooltip or a menu: "Signal: not measured (…)" (#269).
pub fn short_line(code: &str, french: bool) -> String {
    let why = match (code, french) {
        (CODE_NOT_IN_GROUP, false) => "account not in the group akm",
        (CODE_NOT_IN_GROUP, true) => "compte hors du groupe akm",
        (CODE_NEEDS_RELOGIN, false) => "restart the computer",
        (CODE_NEEDS_RELOGIN, true) => "redémarrez l'ordinateur",
        (CODE_HELPER_MISSING, false) => "rssi-helper not installed",
        (CODE_HELPER_MISSING, true) => "rssi-helper non installé",
        (_, false) => "see akmctl doctor",
        (_, true) => "voir akmctl doctor",
    };
    if french {
        format!("Signal\u{a0}: non mesuré ({why})")
    } else {
        format!("Signal: not measured ({why})")
    }
}

/// Members of `group` in an `/etc/group` text.
pub fn group_members<'a>(etc_group: &'a str, group: &str) -> Vec<&'a str> {
    etc_group
        .lines()
        .find_map(|l| {
            let mut it = l.split(':');
            (it.next() == Some(group)).then(|| it.nth(2).unwrap_or(""))
        })
        .map(|m| m.split(',').filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// The issue for one error. `listed_in_akm`: `/etc/group` lists the user in
/// `akm` (asked only for a refusal).
pub fn classify(e: &RssiError, listed_in_akm: impl FnOnce() -> bool) -> RssiIssue {
    let code = match e {
        RssiError::Denied if listed_in_akm() => CODE_NEEDS_RELOGIN,
        RssiError::Denied => CODE_NOT_IN_GROUP,
        RssiError::HelperMissing(_) => CODE_HELPER_MISSING,
        RssiError::Helper { .. } | RssiError::HelperSpawn(_) | RssiError::BadOutput(_) => {
            CODE_HELPER_FAILED
        }
        RssiError::Timeout => CODE_TIMEOUT,
        RssiError::Unavailable => CODE_UNAVAILABLE,
        RssiError::BadMac => CODE_OTHER,
    };
    RssiIssue {
        code: code.to_string(),
        detail: e.to_string(),
    }
}

/// Name of the user running this process.
fn user_name() -> Option<String> {
    if let Some(u) = std::env::var("USER").ok().filter(|u| !u.is_empty()) {
        return Some(u);
    }
    // SAFETY: getuid never fails; getpwuid returns NULL or a static entry
    // whose name is copied at once.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        Some(
            std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// `/etc/group` lists the user of this process in `akm`.
pub fn user_listed_in_akm() -> bool {
    let Some(user) = user_name() else {
        return false;
    };
    std::fs::read_to_string("/etc/group")
        .map(|g| group_members(&g, "akm").contains(&user.as_str()))
        .unwrap_or(false)
}

/// Why the last `read_rssi` failed, `None` when it succeeded (or never ran).
pub fn last_issue() -> Option<RssiIssue> {
    last_error().map(|e| classify(&e, user_listed_in_akm))
}

/// Read RSSI and TX power (dBm) for `mac` (`"AA:BB:CC:DD:EE:FF"`).
/// Returns `None` on any failure; see `last_error()` for the reason.
pub fn read_rssi(mac: &str) -> Option<(i8, Option<i8>)> {
    let res = read_rssi_cached(mac);
    let mut last = LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner());
    match &res {
        Ok(_) => *last = None,
        Err(e) => {
            // Once per distinct cause, at warning priority (#269).
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
fn spawn_error(e: &std::io::Error) -> RssiError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        RssiError::Denied
    } else {
        RssiError::HelperSpawn(e.to_string())
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
        .map_err(|e| spawn_error(&e))?;
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

    /// #269: a helper this process may not run (the package installs it
    /// `root:akm 0750`) is a refusal with a code, not a generic failure.
    #[test]
    fn helper_not_executable_is_a_refusal_with_a_code() {
        let p = fake_helper("noexec", "exit 0");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        // root runs anything: the refusal cannot be reproduced as root.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let e = run_helper(&p, MAC, Duration::from_secs(2)).unwrap_err();
        assert_eq!(e, RssiError::Denied);
        assert_eq!(classify(&e, || false).code, CODE_NOT_IN_GROUP);
        assert_eq!(classify(&e, || true).code, CODE_NEEDS_RELOGIN);
        assert!(classify(&e, || false).detail.contains("usermod -aG akm"));
    }

    #[test]
    fn every_error_has_a_stable_code() {
        let c = |e: RssiError| classify(&e, || panic!("asked only for a refusal")).code;
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
    }

    #[test]
    fn issue_json_is_code_and_detail() {
        let i = classify(&RssiError::Timeout, || false);
        let j = serde_json::to_value(&i).unwrap();
        assert_eq!(j["code"], "timeout");
        assert_eq!(j["detail"], "rssi-helper timed out");
        let back: RssiIssue = serde_json::from_str(r#"{"code":"x"}"#).unwrap();
        assert_eq!(back.detail, "");
    }

    #[test]
    fn every_code_is_explained_with_an_action() {
        for c in [
            CODE_NOT_IN_GROUP,
            CODE_NEEDS_RELOGIN,
            CODE_HELPER_MISSING,
            CODE_HELPER_FAILED,
            CODE_TIMEOUT,
            CODE_UNAVAILABLE,
            CODE_OTHER,
        ] {
            for fr in [false, true] {
                let (why, fix) = explain(c, fr);
                assert!(!why.is_empty() && !fix.is_empty(), "{c}");
            }
        }
        assert!(explain(CODE_NOT_IN_GROUP, true)
            .1
            .contains("usermod -aG akm"));
        // Lingering user manager: a new login is not enough (#269).
        assert!(explain(CODE_NEEDS_RELOGIN, true)
            .1
            .contains("redémarrez l'ordinateur"));
        assert_ne!(
            explain(CODE_NEEDS_RELOGIN, true),
            explain(CODE_NEEDS_RELOGIN, false)
        );
    }

    #[test]
    fn group_members_of_etc_group() {
        let g = "root:x:0:\nakm:x:964:paul,anne\nwheel:x:998:paul\n";
        assert_eq!(group_members(g, "akm"), vec!["paul", "anne"]);
        assert!(group_members(g, "nope").is_empty());
        assert!(group_members("akm:x:964:\n", "akm").is_empty());
    }
}
