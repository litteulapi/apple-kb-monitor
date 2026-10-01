//! The circuit breaker of the daemon, published for the other emitters (#244,
//! #251; `docs/PARITE-APPLE.md` R3, `docs/VEILLE-HID.md`).
//!
//! Apple's rule R3: after three consecutive silences **nothing** goes out to
//! the keyboard any more, HID_CONTROL included, until a new connection or a
//! sleep. The breaker lives in the daemon (`read_policy::Breaker`), but two
//! other programs emit on their own: `akm-hid-control` (root, system units,
//! SUSPEND `0x13` / EXIT_SUSPEND `0x14`) and `akmctl` (`WriteDoor`: `0x41`
//! forget, `0x55` name). They have no access to the daemon's memory, so the
//! daemon **publishes** its breaker as one small text file:
//!
//! ```text
//! $XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state      (= /run/user/<uid>/…)
//! schema=1
//! mac=04:DB:56:CA:42:EE
//! open=1
//! counter=3
//! written_unix=1790000000
//! pid=4242
//! ```
//!
//! Why a file and not D-Bus or a socket: the readers run as root from system
//! units at sleep time, with `RestrictAddressFamilies=AF_UNIX` and no session
//! bus; the helper stays `libc`-only; `/run/user/<uid>` already carries the
//! HID lock and the keymap file the root helpers read (`akm-keymap-helper`),
//! with the same checks (owner = `<uid>` of the path, regular file, no
//! symlink, single link, ≤ 1 KiB). The file is rewritten atomically (temp +
//! rename) on every change and at least every [`REFRESH`] as a heartbeat.
//!
//! Decision of a reader ([`verdict`]), per keyboard:
//!
//! * state **open** for this keyboard -> refuse (`BreakerOpen`);
//! * state older than [`STALE_AFTER`] **and** the daemon still runs -> refuse
//!   (the daemon is wedged, its breaker may be open: fail closed);
//! * file unreadable / malformed **and** the daemon runs -> refuse;
//! * no file, or a dead daemon -> emit (the behaviour before this module);
//! * state of another keyboard, or no keyboard followed -> emit.
//!
//! This file is `std` + `libc` only: `akm-helper` compiles it in by path.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// File name under the runtime directory of the daemon's user.
pub const FILE_NAME: &str = "breaker.state";
/// Directory under `/run/user/<uid>` (same as the HID lock and the keymap file).
pub const DIR_NAME: &str = "apple-kb-monitor";
/// Root of the per-user runtime directories a root reader scans.
pub const RUN_USER_ROOT: &str = "/run/user";
/// Format version.
pub const SCHEMA: u32 = 1;
/// A state older than this, from a daemon that still runs, is not trusted.
pub const STALE_AFTER: Duration = Duration::from_secs(60);
/// The daemon rewrites the file at least this often (heartbeat), well under
/// [`STALE_AFTER`].
pub const REFRESH: Duration = Duration::from_secs(20);
/// Largest file accepted by a reader.
pub const MAX_LEN: u64 = 1024;
/// `comm` of the daemon as the kernel truncates it (TASK_COMM_LEN - 1 = 15).
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
    /// Unix time of the write.
    pub written_unix: u64,
    /// Pid of the daemon (to tell a live daemon from a stale file).
    pub pid: u32,
}

impl BreakerState {
    /// The text form (one `key=value` per line, fixed order).
    pub fn render(&self) -> String {
        format!(
            "schema={SCHEMA}\nmac={}\nopen={}\ncounter={}\nwritten_unix={}\npid={}\n",
            self.mac.as_deref().unwrap_or("-"),
            u8::from(self.open),
            self.counter,
            self.written_unix,
            self.pid
        )
    }

    /// Strict parse: every key once, no unknown key, schema [`SCHEMA`].
    pub fn parse(s: &str) -> Result<Self, String> {
        let (mut schema, mut mac, mut open, mut counter, mut written, mut pid) =
            (None, None, None, None, None, None);
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
        if schema != Some(u64::from(SCHEMA)) {
            return Err(format!("schema {schema:?}, expected {SCHEMA}"));
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
        })
    }

    /// Age of the state at `now_unix` (0 for a state from the future).
    pub fn age(&self, now_unix: u64) -> Duration {
        Duration::from_secs(now_unix.saturating_sub(self.written_unix))
    }

    pub fn is_stale(&self, now_unix: u64) -> bool {
        self.age(now_unix) > STALE_AFTER
    }

    /// Does this state speak about keyboard `mac`?
    pub fn concerns(&self, mac: &str) -> bool {
        self.mac
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case(mac.trim()))
    }
}

/// `-` = no keyboard; else strict upper-cased `XX:XX:XX:XX:XX:XX`.
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

// ── the decision ───────────────────────────────────────────────────────────

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
    pub fn allows(&self) -> bool {
        matches!(self, Verdict::Allow(_))
    }
}

/// One published state as a reader found it: `Ok(None)` = no file,
/// `Err` = unreadable or malformed.
pub type Found = Result<Option<BreakerState>, String>;

/// The decision for keyboard `mac` from one found state; `daemon_alive(pid)`
/// says whether the daemon that wrote it (`Some(pid)`), or any daemon of that
/// user (`None`), is running.
pub fn verdict(
    found: &Found,
    mac: &str,
    now_unix: u64,
    daemon_alive: &dyn Fn(Option<u32>) -> bool,
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
            if st.is_stale(now_unix) {
                return if daemon_alive(Some(st.pid)) {
                    Verdict::Refuse(Refuse::Stale {
                        age_s: st.age(now_unix).as_secs(),
                    })
                } else {
                    Verdict::Allow(Allow::NoDaemon)
                };
            }
            if st.open {
                // A fresh "open" from a daemon that died a second ago is still
                // Apple's verdict for this connection: refused as well.
                Verdict::Refuse(Refuse::Open {
                    counter: st.counter,
                })
            } else {
                Verdict::Allow(Allow::Closed)
            }
        }
    }
}

/// Several users may run a daemon (multi-seat): one refusal is enough.
pub fn combine(verdicts: impl IntoIterator<Item = Verdict>) -> Verdict {
    let mut out = Verdict::Allow(Allow::NoState);
    for v in verdicts {
        match (&out, &v) {
            (Verdict::Refuse(_), _) => {}
            (_, Verdict::Refuse(_)) => out = v,
            (Verdict::Allow(Allow::NoState), Verdict::Allow(_)) => out = v,
            _ => {}
        }
    }
    out
}

// ── is the daemon running? (std only, `/proc`) ─────────────────────────────

/// `Uid:` (real) of `/proc/<pid>/status`.
fn proc_uid(proc_root: &Path, pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(proc_root.join(pid.to_string()).join("status")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn proc_is_daemon(proc_root: &Path, pid: u32, uid: u32) -> bool {
    let comm =
        std::fs::read_to_string(proc_root.join(pid.to_string()).join("comm")).unwrap_or_default();
    comm.trim_end() == DAEMON_COMM && proc_uid(proc_root, pid) == Some(uid)
}

/// Is `apple-kb-monitord` running as `uid`? With `Some(pid)` only that
/// process is checked (the writer of the state), with `None` every process
/// of `/proc` is looked at (the state could not name its writer).
pub fn daemon_alive_in(proc_root: &Path, pid: Option<u32>, uid: u32) -> bool {
    match pid {
        Some(p) => proc_is_daemon(proc_root, p, uid),
        None => std::fs::read_dir(proc_root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .any(|p| proc_is_daemon(proc_root, p, uid)),
    }
}

// ── the writer (daemon) ────────────────────────────────────────────────────

/// Path of the state of the current user (`$XDG_RUNTIME_DIR`, else
/// `/run/user/<uid>`).
pub fn own_path() -> PathBuf {
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x),
        // SAFETY: getuid(2) has no preconditions.
        None => PathBuf::from(RUN_USER_ROOT).join(unsafe { libc::getuid() }.to_string()),
    };
    dir.join(DIR_NAME).join(FILE_NAME)
}

/// Path of the state of user `uid` as a root reader sees it.
pub fn path_for_uid(run_user_root: &Path, uid: u32) -> PathBuf {
    run_user_root
        .join(uid.to_string())
        .join(DIR_NAME)
        .join(FILE_NAME)
}

/// Write `st` at `path` atomically (temp + rename, 0644: the directory is
/// the user's private `0700` one; root reads through `CAP_DAC_READ_SEARCH`).
/// The parent directory must exist (the daemon creates it for the HID lock).
pub fn write(path: &Path, st: &BreakerState) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
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

/// Remove the state (daemon exit): nothing published = the former behaviour
/// of the readers.
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Keeps the daemon's writes rare: a rewrite only on a change or after
/// [`REFRESH`] (heartbeat).
#[derive(Debug, Default)]
pub struct Publisher {
    last: Option<(String, std::time::Instant)>,
}

impl Publisher {
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Should `st` be written now (content changed or heartbeat due)? Pure.
    pub fn due(&self, st: &BreakerState, now: std::time::Instant) -> bool {
        let key = Self::key(st);
        match &self.last {
            None => true,
            Some((k, t)) => *k != key || now.saturating_duration_since(*t) >= REFRESH,
        }
    }

    /// The fields that matter for a change (the timestamp does not).
    fn key(st: &BreakerState) -> String {
        format!("{:?}|{}|{}|{}", st.mac, st.open, st.counter, st.pid)
    }

    /// Write if due; returns whether a write happened. Errors are returned
    /// after being recorded as "written" to avoid a tight retry loop.
    pub fn publish(
        &mut self,
        path: &Path,
        st: &BreakerState,
        now: std::time::Instant,
    ) -> io::Result<bool> {
        if !self.due(st, now) {
            return Ok(false);
        }
        self.last = Some((Self::key(st), now));
        write(path, st).map(|()| true)
    }
}

/// Read the state of the current user (`akmctl`): no owner check beyond
/// the regular-file / no-symlink / size ones (the directory is ours).
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

/// Unix time now.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const KB: &str = "04:DB:56:CA:42:EE";

    fn st(open: bool, counter: u32, written: u64) -> BreakerState {
        BreakerState {
            mac: Some(KB.into()),
            open,
            counter,
            written_unix: written,
            pid: 4242,
        }
    }

    #[test]
    fn render_and_parse_round_trip_and_strictness() {
        let s = st(true, 3, 1_790_000_000);
        let text = s.render();
        assert_eq!(
            text,
            "schema=1\nmac=04:DB:56:CA:42:EE\nopen=1\ncounter=3\nwritten_unix=1790000000\npid=4242\n"
        );
        assert_eq!(BreakerState::parse(&text), Ok(s.clone()));
        let none = BreakerState {
            mac: None,
            ..s.clone()
        };
        assert_eq!(BreakerState::parse(&none.render()), Ok(none));
        assert_eq!(
            BreakerState::parse(
                "schema=1\nmac=04:db:56:ca:42:ee\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n"
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
            "schema=1\nmac=04:DB:56:CA:42\nopen=0\ncounter=0\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=0\ncounter=-1\nwritten_unix=5\npid=1\n",
            "schema=1\nmac=-\nopen=0\ncounter=99999999999\nwritten_unix=5\npid=1\n",
            "garbage",
        ] {
            assert!(BreakerState::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn age_staleness_and_concern() {
        let s = st(false, 0, 1000);
        assert_eq!(s.age(1000), Duration::ZERO);
        assert_eq!(s.age(900), Duration::ZERO, "future write = fresh");
        assert!(!s.is_stale(1000 + STALE_AFTER.as_secs()));
        assert!(s.is_stale(1001 + STALE_AFTER.as_secs()));
        assert!(s.concerns("04:db:56:ca:42:ee") && s.concerns(KB));
        assert!(!s.concerns("11:22:33:44:55:66"));
        assert!(!BreakerState { mac: None, ..s }.concerns(KB));
        assert!(REFRESH < STALE_AFTER);
    }

    #[test]
    fn verdict_follows_the_rule_table() {
        let alive = |_: Option<u32>| true;
        let dead = |_: Option<u32>| false;
        let now = 20_000;
        // open, fresh: refused whatever the daemon's fate
        let open: Found = Ok(Some(st(true, 3, now - 5)));
        assert_eq!(
            verdict(&open, KB, now, &alive),
            Verdict::Refuse(Refuse::Open { counter: 3 })
        );
        assert_eq!(
            verdict(&open, KB, now, &dead),
            Verdict::Refuse(Refuse::Open { counter: 3 })
        );
        // closed, fresh: allowed
        let closed: Found = Ok(Some(st(false, 1, now - 5)));
        assert_eq!(
            verdict(&closed, KB, now, &alive),
            Verdict::Allow(Allow::Closed)
        );
        // stale: refused if the daemon runs, allowed if it is gone
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
        // other keyboard / none followed: allowed
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
        // no file: allowed; unreadable: refused only with a live daemon
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
        // the pid handed to the liveness check is the writer's
        let seen = std::cell::Cell::new(None);
        let spy = |p: Option<u32>| {
            seen.set(Some(p));
            true
        };
        verdict(&stale, KB, now, &spy);
        assert_eq!(seen.get(), Some(Some(4242)));
        verdict(&bad, KB, now, &spy);
        assert_eq!(seen.get(), Some(None));
        for r in [
            Refuse::Open { counter: 3 },
            Refuse::Stale { age_s: 61 },
            Refuse::Unreadable("x".into()),
        ] {
            assert!(
                r.to_string().contains("nothing") || r.to_string().contains("not"),
                "{r}"
            );
        }
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

    #[test]
    fn daemon_liveness_from_a_fake_proc() {
        let root = std::env::temp_dir().join(format!("akm-breaker-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |pid: u32, comm: &str, uid: u32| {
            let d = root.join(pid.to_string());
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(
                d.join("status"),
                format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
        };
        mk(10, "apple-kb-monito", 1000);
        mk(11, "sleep", 1000);
        mk(12, "apple-kb-monito", 1001);
        std::fs::create_dir_all(root.join("self")).unwrap();
        assert!(daemon_alive_in(&root, Some(10), 1000));
        assert!(!daemon_alive_in(&root, Some(11), 1000), "another program");
        assert!(
            !daemon_alive_in(&root, Some(12), 1000),
            "another user's daemon"
        );
        assert!(!daemon_alive_in(&root, Some(99), 1000), "no such process");
        assert!(daemon_alive_in(&root, None, 1000));
        assert!(daemon_alive_in(&root, None, 1001));
        assert!(!daemon_alive_in(&root, None, 1002));
        assert!(!daemon_alive_in(Path::new("/nonexistent-akm"), None, 1000));
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
        // malformed content is an error, not a state
        std::fs::write(&p, "schema=7\n").unwrap();
        assert!(read_own(&p).is_err());
        remove(&p);
        assert_eq!(read_own(&p), Ok(None));
        remove(&p); // idempotent
                    // publisher: first write, then only on change or after REFRESH
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
                t0 + REFRESH - Duration::from_secs(1)
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
