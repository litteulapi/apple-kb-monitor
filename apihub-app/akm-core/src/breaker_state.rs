//! The circuit breaker of the daemon, published for the other emitters.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// File name under the runtime directory of the daemon's user.
pub const FILE_NAME: &str = "breaker.state";
/// Directory under `/run/user/<uid>` (same as the HID lock and the keymap file).
pub const DIR_NAME: &str = "apple-kb-monitor";
/// Root of the per-user runtime directories a root reader scans.
pub const RUN_USER_ROOT: &str = crate::paths::RUN_USER_ROOT;
/// Format version.
pub const SCHEMA: u32 = 2;
/// Former format (no `starttime`), still read.
pub const SCHEMA_V1: u32 = 1;
/// A state dated further than this in the future is not trusted.
pub const CLOCK_SKEW: Duration = Duration::from_secs(5);
/// Executable of the daemon as `/proc/<pid>/exe` shows it.
pub const DAEMON_EXE: &str = "/usr/bin/apple-kb-monitord";
/// A state older than this, from a daemon that still runs, is not trusted.
pub const STALE_AFTER: Duration = Duration::from_mins(1);
/// The daemon rewrites the file at least this often (heartbeat), well under [`STALE_AFTER`].
pub const REFRESH: Duration = Duration::from_secs(20);
/// Largest file accepted by a reader.
pub const MAX_LEN: u64 = 1024;
/// `comm` of the daemon as the kernel truncates it (`TASK_COMM_LEN` - 1 = 15).
pub const DAEMON_COMM: &str = "apple-kb-monito";

/// What the daemon publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakerState {
    /// Keyboard followed (upper-case `XX:XX:XX:XX:XX:XX`), `None` = none.
    pub mac: Option<String>,
    /// R3 flag: nothing may go out.
    pub open: bool,
    /// Consecutive silences so far.
    pub counter: u32,
    /// Write time on the awake clock ([`now_unix`]): Unix time minus the time suspended since boot; not a date.
    pub written_unix: u64,
    /// Pid of the daemon (to tell a live daemon from a stale file).
    pub pid: u32,
    /// Start time of the daemon: a recycled pid does not match it.
    pub starttime: Option<u64>,
}

/// The writer of a state as the liveness check sees it: pid and, when known, its start time.
pub type Writer = (u32, Option<u64>);

impl BreakerState {
    /// The text form (one `key=value` per line, fixed order).
    #[must_use]
    pub fn render(&self) -> String {
        let mut s = format!(
            "schema={}\nmac={}\nopen={}\ncounter={}\nwritten_unix={}\npid={}\n",
            if self.starttime.is_some() {
                SCHEMA
            } else {
                SCHEMA_V1
            },
            self.mac.as_deref().unwrap_or("-"),
            u8::from(self.open),
            self.counter,
            self.written_unix,
            self.pid
        );
        if let Some(t) = self.starttime {
            let _ = writeln!(s, "starttime={t}");
        }
        s
    }

    /// Strict parse: every key once, no unknown key, schema [`SCHEMA`] or [`SCHEMA_V1`].
    ///
    /// # Errors
    ///
    /// A message naming the first offending line or key.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (mut schema, mut mac, mut open, mut counter, mut written, mut pid) =
            (None, None, None, None, None, None);
        let mut starttime = None;
        for (i, raw) in s.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: expected key=value", i + 1))?;
            let v = v.trim();
            let num = |what: &str| -> Result<u64, String> {
                if v.is_empty() || v.len() > 12 || !v.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(format!("line {}: invalid {what} {v:?}", i + 1));
                }
                v.parse::<u64>()
                    .map_err(|_| format!("line {}: invalid {what} {v:?}", i + 1))
            };
            let slot: &mut Option<u64> = match k.trim() {
                "schema" => &mut schema,
                "open" => &mut open,
                "counter" => &mut counter,
                "written_unix" => &mut written,
                "pid" => &mut pid,
                "starttime" => &mut starttime,
                "mac" => {
                    if mac.replace(parse_mac(v)?).is_some() {
                        return Err(format!("line {}: mac given twice", i + 1));
                    }
                    continue;
                }
                other => return Err(format!("line {}: unknown key {other:?}", i + 1)),
            };
            let n = num(k.trim())?;
            if slot.replace(n).is_some() {
                return Err(format!("line {}: {} given twice", i + 1, k.trim()));
            }
        }
        match (schema, starttime) {
            (Some(s), Some(_)) if s == u64::from(SCHEMA) => {}
            (Some(s), None) if s == u64::from(SCHEMA_V1) => {}
            (Some(s), _) if s == u64::from(SCHEMA) => return Err("starttime missing".into()),
            (Some(s), _) if s == u64::from(SCHEMA_V1) => {
                return Err(format!("starttime in schema {SCHEMA_V1}"))
            }
            _ => {
                return Err(format!(
                    "schema {schema:?}, expected {SCHEMA_V1} or {SCHEMA}"
                ))
            }
        }
        let open = match open {
            Some(0) => false,
            Some(1) => true,
            o => return Err(format!("open {o:?}, expected 0 or 1")),
        };
        let counter = counter.ok_or("counter missing")?;
        let pid = pid.ok_or("pid missing")?;
        Ok(Self {
            mac: mac.ok_or("mac missing")?,
            open,
            counter: u32::try_from(counter).map_err(|_| "counter too large")?,
            written_unix: written.ok_or("written_unix missing")?,
            pid: u32::try_from(pid).map_err(|_| "pid too large")?,
            starttime,
        })
    }

    /// Age of the state at `now_unix`.
    #[must_use]
    pub fn age(&self, now_unix: u64) -> Duration {
        Duration::from_secs(now_unix.saturating_sub(self.written_unix))
    }

    #[must_use]
    pub fn is_stale(&self, now_unix: u64) -> bool {
        self.age(now_unix) > STALE_AFTER
    }

    /// Seconds the state is dated ahead of `now_unix`, when that is more than [`CLOCK_SKEW`];
    /// `None` = plausible date.
    #[must_use]
    pub fn ahead_of(&self, now_unix: u64) -> Option<u64> {
        let ahead = self.written_unix.saturating_sub(now_unix);
        (ahead > CLOCK_SKEW.as_secs()).then_some(ahead)
    }

    /// The writer, for the liveness check.
    #[must_use]
    pub fn writer(&self) -> Writer {
        (self.pid, self.starttime)
    }

    /// Does this state speak about keyboard `mac`?
    #[must_use]
    pub fn concerns(&self, mac: &str) -> bool {
        self.mac
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case(mac.trim()))
    }
}

fn parse_mac(v: &str) -> Result<Option<String>, String> {
    if v == "-" {
        return Ok(None);
    }
    let ok = v.len() == 17
        && v.bytes().enumerate().all(|(i, b)| {
            if i % 3 == 2 {
                b == b':'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if ok {
        Ok(Some(v.to_ascii_uppercase()))
    } else {
        Err(format!("invalid mac {v:?}"))
    }
}

/// Why an emission may go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allow {
    /// No published state at all (no daemon, or an older one).
    NoState,
    /// A state exists but its daemon is gone: the breaker died with it.
    NoDaemon,
    /// The daemon follows another keyboard (or none).
    OtherKeyboard,
    /// The daemon's breaker is closed.
    Closed,
}

/// Why nothing goes out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refuse {
    /// R3: the breaker is open (`counter` silences).
    Open { counter: u32 },
    /// The state is older than [`STALE_AFTER`] while the daemon runs.
    Stale { age_s: u64 },
    /// The state cannot be read or parsed while the daemon runs.
    Unreadable(String),
    /// The state is dated `ahead_s` seconds in the future while its daemon runs.
    Future { ahead_s: u64 },
    /// The root reader does not know whose breaker to read: fail closed.
    NoActiveUser,
}

impl std::fmt::Display for Refuse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { counter } => write!(
                f,
                "breaker open: the keyboard did not answer {counter} requests in a row (Apple R3: nothing is sent until a new connection or a sleep)"
            ),
            Self::Stale { age_s } => write!(
                f,
                "breaker state is {age_s} s old (> {} s) while the daemon runs: not trusted, nothing sent",
                STALE_AFTER.as_secs()
            ),
            Self::Unreadable(e) => {
                write!(f, "breaker state unreadable while the daemon runs ({e}): nothing sent")
            }
            Self::Future { ahead_s } => write!(
                f,
                "breaker state dated {ahead_s} s in the future (> {} s of clock skew) while the daemon runs: not trusted, nothing sent",
                CLOCK_SKEW.as_secs()
            ),
            Self::NoActiveUser => write!(
                f,
                "no active user on the seat: whose breaker applies is unknown, nothing sent"
            ),
        }
    }
}

/// Verdict of a reader for one keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow(Allow),
    Refuse(Refuse),
}

impl Verdict {
    #[must_use]
    pub fn allows(&self) -> bool {
        matches!(self, Verdict::Allow(_))
    }
}

/// One published state as a reader found it: `Ok(None)` = no file, `Err` = unreadable or malformed.
pub type Found = Result<Option<BreakerState>, String>;

/// The decision for keyboard `mac` from one found state.
pub fn verdict(
    found: &Found,
    mac: &str,
    now_unix: u64,
    daemon_alive: &dyn Fn(Option<Writer>) -> bool,
) -> Verdict {
    match found {
        Err(e) => {
            if daemon_alive(None) {
                Verdict::Refuse(Refuse::Unreadable(e.clone()))
            } else {
                Verdict::Allow(Allow::NoDaemon)
            }
        }
        Ok(None) => Verdict::Allow(Allow::NoState),
        Ok(Some(st)) => {
            if !st.concerns(mac) {
                return Verdict::Allow(Allow::OtherKeyboard);
            }
            if let Some(ahead_s) = st.ahead_of(now_unix) {
                // Never fresh: fail closed while its writer runs, ignored once it is gone.
                return if daemon_alive(Some(st.writer())) {
                    Verdict::Refuse(Refuse::Future { ahead_s })
                } else {
                    Verdict::Allow(Allow::NoDaemon)
                };
            }
            if st.is_stale(now_unix) {
                return if daemon_alive(Some(st.writer())) {
                    Verdict::Refuse(Refuse::Stale {
                        age_s: st.age(now_unix).as_secs(),
                    })
                } else {
                    Verdict::Allow(Allow::NoDaemon)
                };
            }
            if st.open {
                // A fresh "open" from a daemon that just died still holds, for STALE_AFTER at most.
                Verdict::Refuse(Refuse::Open {
                    counter: st.counter,
                })
            } else {
                Verdict::Allow(Allow::Closed)
            }
        }
    }
}

#[cfg(test)]
/// Combines verdicts: one refusal is enough.
pub fn combine(verdicts: impl IntoIterator<Item = Verdict>) -> Verdict {
    let mut out = Verdict::Allow(Allow::NoState);
    for v in verdicts {
        match (&out, &v) {
            (Verdict::Refuse(_), _) => {}
            (_, Verdict::Refuse(_)) | (Verdict::Allow(Allow::NoState), Verdict::Allow(_)) => {
                out = v;
            }
            _ => {}
        }
    }
    out
}

fn proc_uid(proc_root: &Path, pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(proc_root.join(pid.to_string()).join("status")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Start time of `pid` (`/proc/<pid>/stat` field 22, clock ticks since boot).
#[must_use]
pub fn proc_starttime(proc_root: &Path, pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    let rest = &s[s.rfind(')')? + 1..];
    // after ")": field 3 (state) is index 0, field 22 (starttime) index 19
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn proc_exe_is_daemon(proc_root: &Path, pid: u32) -> bool {
    std::fs::read_link(proc_root.join(pid.to_string()).join("exe")).is_ok_and(|p| {
        let p = p.to_string_lossy();
        p == DAEMON_EXE || p.strip_suffix(" (deleted)") == Some(DAEMON_EXE)
    })
}

/// `pid` is `apple-kb-monitord` run by `uid` (exe + real uid, never `comm`), and, if `starttime` is
/// given, the very process that wrote the state.
fn proc_is_daemon(proc_root: &Path, pid: u32, starttime: Option<u64>, uid: u32) -> bool {
    proc_uid(proc_root, pid) == Some(uid)
        && proc_exe_is_daemon(proc_root, pid)
        && starttime.is_none_or(|t| proc_starttime(proc_root, pid) == Some(t))
}

/// Is `apple-kb-monitord` running as `uid`?
#[must_use]
pub fn daemon_alive_in(proc_root: &Path, writer: Option<Writer>, uid: u32) -> bool {
    match writer {
        Some((p, t)) => proc_is_daemon(proc_root, p, t, uid),
        None => std::fs::read_dir(proc_root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .any(|p| proc_is_daemon(proc_root, p, None, uid)),
    }
}

/// `ACTIVE_UID=` of logind's seat file (`/run/systemd/seats/seat0`).
#[must_use]
pub fn seat_active_uid(seat_file: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(seat_file).ok()?;
    let v = s
        .lines()
        .find_map(|l| l.strip_prefix("ACTIVE_UID="))?
        .trim();
    if v.is_empty() || v.len() > 10 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse().ok()
}

/// Seat file read by the root reader.
pub const SEAT0: &str = "/run/systemd/seats/seat0";

#[cfg(test)]
/// Path of the state of the current user (`$XDG_RUNTIME_DIR`, else `/run/user/<uid>`).
#[must_use]
pub fn own_path() -> PathBuf {
    crate::paths::runtime_home().join(DIR_NAME).join(FILE_NAME)
}

#[must_use]
pub fn path_for_uid(run_user_root: &Path, uid: u32) -> PathBuf {
    run_user_root
        .join(uid.to_string())
        .join(DIR_NAME)
        .join(FILE_NAME)
}

/// Write `st` at `path` atomically.
// Own copy, not fsutil::write_atomic: user-owned 0600 state read by root, a symlinked target is refused.
///
/// # Errors
///
/// Any I/O error while writing or renaming the file.
pub fn write(path: &Path, st: &BreakerState) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let mut tmp_name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp)?;
    f.write_all(st.render().as_bytes())?;
    f.sync_data()?;
    drop(f);
    if let Ok(md) = std::fs::symlink_metadata(path) {
        if !md.file_type().is_file() {
            let _ = std::fs::remove_file(&tmp);
            return Err(io::Error::other(format!(
                "{} is not a regular file (symlink?): not replaced",
                path.display()
            )));
        }
    }
    std::fs::rename(&tmp, path)
}

/// Remove the state (daemon exit): nothing published = the former behaviour of the readers.
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Keeps the daemon's writes rare: a rewrite only on a change or after [`REFRESH`] (heartbeat).
#[derive(Debug, Default)]
pub struct Publisher {
    last: Option<(String, std::time::Instant)>,
}

impl Publisher {
    #[must_use]
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Should `st` be written now (content changed or heartbeat due)?
    #[must_use]
    pub fn due(&self, st: &BreakerState, now: std::time::Instant) -> bool {
        let key = Self::key(st);
        match &self.last {
            None => true,
            Some((k, t)) => *k != key || now.saturating_duration_since(*t) >= REFRESH,
        }
    }

    fn key(st: &BreakerState) -> String {
        format!("{:?}|{}|{}|{}", st.mac, st.open, st.counter, st.pid)
    }

    /// Write if due; returns whether a write happened.
    ///
    /// # Errors
    ///
    /// Any I/O error from [`write()`].
    pub fn publish(
        &mut self,
        path: &Path,
        st: &BreakerState,
        now: std::time::Instant,
    ) -> io::Result<bool> {
        if !self.due(st, now) {
            return Ok(false);
        }
        write(path, st)?;
        self.last = Some((Self::key(st), now));
        Ok(true)
    }
}

/// Read the state of the current user (`akmctl`).
///
/// # Errors
///
/// A message when the file exists but cannot be read or parsed.
pub fn read_own(path: &Path) -> Found {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let md = f.metadata().map_err(|e| e.to_string())?;
    if !md.file_type().is_file() || md.len() > MAX_LEN {
        return Err(format!(
            "{}: not a regular file of at most {MAX_LEN} bytes",
            path.display()
        ));
    }
    let mut s = String::new();
    (&mut f)
        .take(MAX_LEN + 1)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    BreakerState::parse(&s)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The clock of `written_unix`: wall time minus the time spent suspended since boot.
///
/// It stops during suspend like the daemon's heartbeat ([`Publisher`], `Instant`), so a state
/// written just before a sleep is still fresh at wake, whatever the sleep lasted.
#[must_use]
pub fn now_unix() -> u64 {
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    awake_unix(
        wall,
        clock(libc::CLOCK_BOOTTIME),
        clock(libc::CLOCK_MONOTONIC),
    )
}

fn awake_unix(wall: Duration, boottime: Duration, monotonic: Duration) -> u64 {
    wall.saturating_sub(boottime.saturating_sub(monotonic))
        .as_secs()
}

fn clock(id: libc::clockid_t) -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid clock id and a valid out pointer.
    if unsafe { libc::clock_gettime(id, &raw mut ts) } != 0 {
        return Duration::ZERO;
    }
    Duration::new(
        u64::try_from(ts.tv_sec).unwrap_or(0),
        u32::try_from(ts.tv_nsec).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const KB: &str = "AA:BB:CC:DD:EE:F1";

    fn st(open: bool, counter: u32, written: u64) -> BreakerState {
        BreakerState {
            mac: Some(KB.into()),
            open,
            counter,
            written_unix: written,
            pid: 4242,
            starttime: Some(777),
        }
    }

    #[test]
    fn render_and_parse_round_trip_and_strictness() {
        let s = st(true, 3, 1_790_000_000);
        let text = s.render();
        assert_eq!(
            text,
            "schema=2\nmac=AA:BB:CC:DD:EE:F1\nopen=1\ncounter=3\nwritten_unix=1790000000\npid=4242\nstarttime=777\n"
        );
        assert_eq!(BreakerState::parse(&text), Ok(s.clone()));
        let none = BreakerState {
            mac: None,
            ..s.clone()
        };
        assert_eq!(BreakerState::parse(&none.render()), Ok(none));
        assert_eq!(
            BreakerState::parse(
                "schema=1\nmac=aa:bb:cc:dd:ee:f1\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n"
            )
            .unwrap()
            .mac
            .as_deref(),
            Some(KB)
        );
        for bad in [
            "",
            "schema=2\nmac=-\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=2\ncounter=0\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=0\ncounter=0\nwritten_unix=5\n",
            "schema=1\nmac=-\nopen=0\ncounter=0\nwritten_unix=5\npid=1\nextra=1\n",
            "schema=1\nmac=-\nopen=0\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=AA:BB:CC:DD:EE\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=0\ncounter=-1\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=0\ncounter=99999999999\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=0\ncounter=0\nwritten_unix=5\npid=1\nstarttime=3\n",
            "schema=2\nmac=-\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n",
            "schema=2\nmac=-\nopen=0\ncounter=0\nwritten_unix=1234567890123\npid=1\nstarttime=3\n",
            "garbage",
        ] {
            assert!(BreakerState::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn schema_1_is_still_read_and_numbers_are_bounded_at_12_digits() {
        let v1 = "schema=1\nmac=-\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n";
        let s = BreakerState::parse(v1).unwrap();
        assert_eq!(s.starttime, None);
        assert_eq!(s.render(), v1, "a schema-1 state renders as schema 1");
        let twelve =
            "schema=2\nmac=-\nopen=0\ncounter=0\nwritten_unix=999999999999\npid=1\nstarttime=3\n";
        assert_eq!(
            BreakerState::parse(twelve).unwrap().written_unix,
            999_999_999_999
        );
    }

    #[test]
    fn age_staleness_and_concern() {
        let s = st(false, 0, 1000);
        assert_eq!(s.age(1000), Duration::ZERO);
        assert_eq!(s.age(900), Duration::ZERO, "future write = fresh");
        assert!(!s.is_stale(1000 + STALE_AFTER.as_secs()));
        assert!(s.is_stale(1001 + STALE_AFTER.as_secs()));
        assert!(s.concerns("aa:bb:cc:dd:ee:f1") && s.concerns(KB));
        assert!(!s.concerns("11:22:33:44:55:66"));
        assert!(!BreakerState { mac: None, ..s }.concerns(KB));
        assert!(REFRESH < STALE_AFTER);
    }

    #[test]
    fn verdict_follows_the_rule_table() {
        let alive = |_: Option<Writer>| true;
        let dead = |_: Option<Writer>| false;
        let now = 20_000;
        let open: Found = Ok(Some(st(true, 3, now - 5)));
        assert_eq!(
            verdict(&open, KB, now, &alive),
            Verdict::Refuse(Refuse::Open { counter: 3 })
        );
        assert_eq!(
            verdict(&open, KB, now, &dead),
            Verdict::Refuse(Refuse::Open { counter: 3 })
        );
        let closed: Found = Ok(Some(st(false, 1, now - 5)));
        assert_eq!(
            verdict(&closed, KB, now, &alive),
            Verdict::Allow(Allow::Closed)
        );
        let stale: Found = Ok(Some(st(true, 3, now - 61)));
        assert_eq!(
            verdict(&stale, KB, now, &alive),
            Verdict::Refuse(Refuse::Stale { age_s: 61 })
        );
        assert_eq!(
            verdict(&stale, KB, now, &dead),
            Verdict::Allow(Allow::NoDaemon)
        );
        let stale_closed: Found = Ok(Some(st(false, 0, now - 3600)));
        assert!(!verdict(&stale_closed, KB, now, &alive).allows());
        assert_eq!(
            verdict(&open, "11:22:33:44:55:66", now, &alive),
            Verdict::Allow(Allow::OtherKeyboard)
        );
        let none: Found = Ok(Some(BreakerState {
            mac: None,
            ..st(true, 3, now)
        }));
        assert_eq!(
            verdict(&none, KB, now, &alive),
            Verdict::Allow(Allow::OtherKeyboard)
        );
        assert_eq!(
            verdict(&Ok(None), KB, now, &alive),
            Verdict::Allow(Allow::NoState)
        );
        let bad: Found = Err("x".into());
        assert_eq!(
            verdict(&bad, KB, now, &alive),
            Verdict::Refuse(Refuse::Unreadable("x".into()))
        );
        assert_eq!(
            verdict(&bad, KB, now, &dead),
            Verdict::Allow(Allow::NoDaemon)
        );
        let seen = std::cell::Cell::new(None);
        let spy = |p: Option<Writer>| {
            seen.set(Some(p));
            true
        };
        verdict(&stale, KB, now, &spy);
        assert_eq!(seen.get(), Some(Some((4242, Some(777)))));
        verdict(&bad, KB, now, &spy);
        assert_eq!(seen.get(), Some(None));
        for r in [
            Refuse::Open { counter: 3 },
            Refuse::Stale { age_s: 61 },
            Refuse::Unreadable("x".into()),
            Refuse::Future { ahead_s: 9 },
            Refuse::NoActiveUser,
        ] {
            assert!(
                r.to_string().contains("nothing") || r.to_string().contains("not"),
                "{r}"
            );
        }
    }

    #[test]
    fn future_dated_state_is_not_trusted_beyond_the_clock_skew() {
        let alive = |_: Option<Writer>| true;
        let dead = |_: Option<Writer>| false;
        let now = 20_000;
        let skew = CLOCK_SKEW.as_secs();
        for open in [true, false] {
            let far: Found = Ok(Some(st(open, 3, now + skew + 1)));
            assert_eq!(
                verdict(&far, KB, now, &dead),
                Verdict::Allow(Allow::NoDaemon),
                "a dead daemon's future state never blocks"
            );
            assert_eq!(
                verdict(&far, KB, now, &alive),
                Verdict::Refuse(Refuse::Future { ahead_s: skew + 1 }),
                "fail closed while its writer runs"
            );
            let year_33658: Found = Ok(Some(st(open, 3, 999_999_999_999)));
            assert!(verdict(&year_33658, KB, now, &dead).allows());
        }
        let near: Found = Ok(Some(st(true, 3, now + skew)));
        assert_eq!(
            verdict(&near, KB, now, &dead),
            Verdict::Refuse(Refuse::Open { counter: 3 })
        );
        assert_eq!(st(false, 0, now + skew).ahead_of(now), None);
        assert_eq!(st(false, 0, now + skew + 1).ahead_of(now), Some(skew + 1));
        assert_eq!(st(false, 0, 0).ahead_of(now), None);
        let seen = std::cell::Cell::new(None);
        let spy = |p: Option<Writer>| {
            seen.set(Some(p));
            false
        };
        verdict(&Ok(Some(st(true, 3, now + 60))), KB, now, &spy);
        assert_eq!(seen.get(), Some(Some((4242, Some(777)))));
        assert_eq!(
            verdict(
                &Ok(Some(st(true, 3, now + 60))),
                "11:22:33:44:55:66",
                now,
                &alive
            ),
            Verdict::Allow(Allow::OtherKeyboard)
        );
    }

    #[test]
    fn allows_is_true_exactly_for_allow() {
        for a in [
            Allow::NoState,
            Allow::NoDaemon,
            Allow::OtherKeyboard,
            Allow::Closed,
        ] {
            assert!(Verdict::Allow(a).allows(), "{a:?}");
        }
        for r in [
            Refuse::Open { counter: 3 },
            Refuse::Stale { age_s: 61 },
            Refuse::Unreadable("x".into()),
            Refuse::Future { ahead_s: 6 },
            Refuse::NoActiveUser,
        ] {
            assert!(!Verdict::Refuse(r).allows());
        }
    }

    #[test]
    fn seat_active_uid_reads_loginds_seat_file_strictly() {
        let dir = std::env::temp_dir().join(format!("akm-breaker-seat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("seat0");
        assert_eq!(seat_active_uid(&f), None, "no file");
        for (text, want) in [
            (
                "IS_SEAT0=1\nACTIVE=c2\nACTIVE_UID=1000\nSESSIONS=c2\n",
                Some(1000),
            ),
            ("ACTIVE_UID=0\n", Some(0)),
            ("IS_SEAT0=1\n", None),
            ("ACTIVE_UID=\n", None),
            ("ACTIVE_UID=-1\n", None),
            ("ACTIVE_UID=10a\n", None),
            ("ACTIVE_UID=12345678901\n", None),
            ("ACTIVE_UID=99999999999\n", None),
        ] {
            std::fs::write(&f, text).unwrap();
            assert_eq!(seat_active_uid(&f), want, "{text:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_refusal_wins_in_a_combination() {
        use Verdict::{Allow as A, Refuse as R};
        assert_eq!(combine([]), A(Allow::NoState));
        assert_eq!(
            combine([A(Allow::NoState), A(Allow::Closed)]),
            A(Allow::Closed)
        );
        assert_eq!(
            combine([A(Allow::Closed), A(Allow::OtherKeyboard)]),
            A(Allow::Closed)
        );
        assert_eq!(
            combine([
                A(Allow::Closed),
                R(Refuse::Open { counter: 3 }),
                A(Allow::NoState)
            ]),
            R(Refuse::Open { counter: 3 })
        );
        assert_eq!(
            combine([
                R(Refuse::Stale { age_s: 70 }),
                R(Refuse::Open { counter: 3 })
            ]),
            R(Refuse::Stale { age_s: 70 }),
            "the first refusal is kept"
        );
    }

    pub(crate) fn fake_proc(root: &Path, pid: u32, exe: &str, comm: &str, uid: u32, start: u64) {
        let d = root.join(pid.to_string());
        std::fs::create_dir_all(&d).unwrap();
        let _ = std::fs::remove_file(d.join("exe"));
        std::os::unix::fs::symlink(exe, d.join("exe")).unwrap();
        std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            d.join("status"),
            format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
        )
        .unwrap();
        // field 3 (state), fields 4..=21, then starttime (22).
        let mid = ["0"; 18].join(" ");
        std::fs::write(
            d.join("stat"),
            format!("{pid} ({comm} x) y) S {mid} {start} 0 0\n"),
        )
        .unwrap();
    }

    #[test]
    fn daemon_liveness_from_a_fake_proc() {
        let root = std::env::temp_dir().join(format!("akm-breaker-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        fake_proc(&root, 10, DAEMON_EXE, "apple-kb-monito", 1000, 500);
        fake_proc(&root, 11, "/usr/bin/sleep", "sleep", 1000, 501);
        fake_proc(&root, 12, DAEMON_EXE, "apple-kb-monito", 1001, 502);
        fake_proc(&root, 13, "/usr/bin/python3", "apple-kb-monito", 1002, 503);
        fake_proc(
            &root,
            14,
            &format!("{DAEMON_EXE} (deleted)"),
            "apple-kb-monito",
            1003,
            504,
        );
        std::fs::create_dir_all(root.join("self")).unwrap();
        assert_eq!(proc_starttime(&root, 10), Some(500));
        assert_eq!(proc_starttime(&root, 99), None);
        assert!(daemon_alive_in(&root, Some((10, None)), 1000));
        assert!(daemon_alive_in(&root, Some((10, Some(500))), 1000));
        assert!(
            !daemon_alive_in(&root, Some((10, Some(499))), 1000),
            "pid recycled: another start time"
        );
        assert!(
            !daemon_alive_in(&root, Some((11, None)), 1000),
            "another program"
        );
        assert!(
            !daemon_alive_in(&root, Some((12, None)), 1000),
            "another user's daemon"
        );
        assert!(
            !daemon_alive_in(&root, Some((13, None)), 1002),
            "comm alone proves nothing"
        );
        assert!(
            !daemon_alive_in(&root, None, 1002),
            "comm alone proves nothing"
        );
        assert!(daemon_alive_in(&root, Some((14, Some(504))), 1003));
        assert!(
            !daemon_alive_in(&root, Some((99, None)), 1000),
            "no such process"
        );
        assert!(daemon_alive_in(&root, None, 1000));
        assert!(daemon_alive_in(&root, None, 1001));
        assert!(!daemon_alive_in(&root, None, 1004));
        assert!(!daemon_alive_in(Path::new("/nonexistent-akm"), None, 1000));
        let me = std::process::id();
        assert!(proc_starttime(Path::new("/proc"), me).is_some());
        // SAFETY: getuid(2) has no preconditions.
        let uid = unsafe { libc::getuid() };
        assert!(!daemon_alive_in(Path::new("/proc"), Some((me, None)), uid));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_read_remove_and_publisher_heartbeat() {
        let dir = std::env::temp_dir().join(format!("akm-breaker-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(FILE_NAME);
        assert_eq!(read_own(&p), Ok(None), "absent = no state");
        let s = st(true, 3, 123);
        write(&p, &s).unwrap();
        assert_eq!(read_own(&p), Ok(Some(s.clone())));
        let mode =
            std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&p).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600, "private to the user");
        assert!(
            !dir.join("breaker.state.tmp").exists(),
            "temp file renamed away"
        );
        // a symlink in place of the file is never followed, never replaced
        let victim = dir.join("victim");
        std::fs::write(&victim, "v").unwrap();
        let link = dir.join("linked.state");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        assert!(write(&link, &s).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "v");
        assert!(read_own(&link).is_err());
        std::fs::write(&p, "schema=7\n").unwrap();
        assert!(read_own(&p).is_err());
        remove(&p);
        assert_eq!(read_own(&p), Ok(None));
        remove(&p); // idempotent
        let mut pb = Publisher::new();
        let t0 = Instant::now();
        assert!(pb.publish(&p, &s, t0).unwrap());
        assert!(
            !pb.publish(
                &p,
                &BreakerState {
                    written_unix: 999,
                    ..s.clone()
                },
                t0 + Duration::from_secs(1)
            )
            .unwrap(),
            "timestamp alone is no change"
        );
        assert!(
            pb.publish(&p, &st(false, 0, 124), t0 + Duration::from_secs(1))
                .unwrap(),
            "change"
        );
        assert!(!pb
            .publish(
                &p,
                &st(false, 0, 125),
                (t0 + REFRESH).checked_sub(Duration::from_secs(1)).unwrap()
            )
            .unwrap());
        assert!(
            pb.publish(
                &p,
                &st(false, 0, 126),
                t0 + Duration::from_secs(1) + REFRESH
            )
            .unwrap(),
            "heartbeat"
        );
        assert_eq!(read_own(&p).unwrap().unwrap().written_unix, 126);
        assert!(path_for_uid(Path::new("/run/user"), 1000)
            .ends_with("1000/apple-kb-monitor/breaker.state"));
        assert!(own_path().ends_with("apple-kb-monitor/breaker.state"));
        let body = st(false, 0, 1).render();
        let pad = |n: usize| format!("{body}{}", "\n".repeat(n - body.len()));
        std::fs::write(&p, pad(usize::try_from(MAX_LEN).unwrap())).unwrap();
        assert!(read_own(&p).unwrap().is_some(), "MAX_LEN bytes accepted");
        std::fs::write(&p, pad(usize::try_from(MAX_LEN).unwrap() + 1)).unwrap();
        assert!(read_own(&p).is_err(), "MAX_LEN + 1 refused");
        // a FIFO never blocks the reader (O_NONBLOCK) and is refused
        let fifo = dir.join("fifo.state");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(read_own(&fifo).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_is_retried_on_the_next_pass() {
        let dir = std::env::temp_dir().join(format!("akm-breaker-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join(FILE_NAME);
        let mut pb = Publisher::new();
        let t0 = Instant::now();
        let open = st(true, 1, 7);
        assert!(pb.publish(&p, &open, t0).is_err(), "directory missing");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(pb.publish(&p, &open, t0 + Duration::from_secs(1)).unwrap());
        assert_eq!(read_own(&p), Ok(Some(open)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_state_written_before_a_long_sleep_is_fresh_at_wake() {
        let dir = std::env::temp_dir().join(format!("akm-breaker-sleep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(FILE_NAME);
        let s = Duration::from_secs;
        // (wall, CLOCK_BOOTTIME, CLOCK_MONOTONIC): 600 s of suspend stop only the last one
        let entry = (s(1_790_000_000), s(5_000), s(4_000));
        let wake = |awake: u64| {
            (
                entry.0 + s(600 + awake),
                entry.1 + s(600 + awake),
                entry.2 + s(awake),
            )
        };
        let now = |c: (Duration, Duration, Duration)| awake_unix(c.0, c.1, c.2);
        let alive = |_: Option<Writer>| true;
        let mut pb = Publisher::new();
        let t0 = Instant::now();
        assert!(pb.publish(&p, &st(false, 0, now(entry)), t0).unwrap());
        let w = wake(2);
        assert!(
            !pb.publish(&p, &st(false, 0, now(w)), t0 + s(2)).unwrap(),
            "no heartbeat due 2 s after wake"
        );
        let found = read_own(&p);
        assert_eq!(
            verdict(&found, KB, now(w), &alive),
            Verdict::Allow(Allow::Closed)
        );
        assert!(
            matches!(
                verdict(&found, KB, w.0.as_secs(), &alive),
                Verdict::Refuse(Refuse::Stale { .. })
            ),
            "the wall clock alone counts the sleep"
        );
        assert!(
            matches!(
                verdict(&found, KB, now(wake(61)), &alive),
                Verdict::Refuse(Refuse::Stale { .. })
            ),
            "a daemon silent while awake is still stale"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
