//! `akmctl selftest`: periodic health check of the installed monitor, run by
//! `apple-kb-monitor-selfcheck.timer` every 15 minutes (docs/TESTING.md).

use akm_core::{tr, trn};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Args;
use serde_json::{json, Value};

use crate::bus;
use crate::cli::{EXIT_ERROR, EXIT_OK};
use crate::doctor;

pub const OUR_EXES: &[&str] = &["apple-kb-monitord", "rssi-helper", "akm-helper", "akmctl"];
const COREDUMP_MSGID: &str = "fc2e22bc6ee647b6b90729ab34a250b1";
/// The daemon must answer `GetState` within this delay.
const DBUS_TIMEOUT: Duration = Duration::from_secs(5);
/// `akmctl doctor` (`BlueZ` + daemon + journal) must finish within this delay.
const DOCTOR_TIMEOUT: Duration = Duration::from_secs(20);

#[allow(clippy::struct_excessive_bools)] // independent command-line flags
#[allow(clippy::doc_markdown)] // doc comments are clap help texts (translated msgids)
#[derive(Args, Debug, Clone)]
pub struct SelftestArgs {
    /// Print the result as JSON (the same object as selfcheck.json)
    #[arg(long)]
    pub json: bool,
    /// Desktop notification for each NEW grave problem (deduplicated)
    #[arg(long)]
    pub notify: bool,
    /// Also open one Gitea issue per new grave problem (deduplicated; needs
    /// AKM_GITEA_API, AKM_GITEA_REPO and ~/.config/gitea/token). Off by default
    #[arg(long)]
    pub gitea_issue: bool,
    /// Result file (default: $XDG_STATE_HOME/apple-kb-monitor/selfcheck.json)
    #[arg(long, value_name = "PATH")]
    pub state_file: Option<PathBuf>,
    /// Do not write the result file (dry run)
    #[arg(long)]
    pub no_save: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Bad,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Bad => "bad",
        }
    }
    fn parse(s: &str) -> Level {
        match s {
            "bad" => Level::Bad,
            "warn" => Level::Warn,
            "info" => Level::Info,
            _ => Level::Ok,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub id: &'static str,
    pub level: Level,
    pub text: String,
    pub key: String,
    /// Stable message id and its values: the System Settings module translates from them.
    pub msg: &'static str,
    pub args: Vec<String>,
}

impl Check {
    fn msg(mut self, msg: &'static str, args: Vec<String>) -> Self {
        self.msg = msg;
        self.args = args;
        self
    }
}

fn versions_check(daemon: Option<&str>, mine: &str, pkg: Option<&str>) -> Check {
    use akm_core::stale::same_version;
    let p = pkg.unwrap_or("?");
    match daemon {
        Some(v) if !same_version(v, mine) || pkg.is_some_and(|p| !same_version(p, v)) => check(
            "versions",
            Level::Warn,
            format!("{v}/{mine}/{}", pkg.unwrap_or("-")),
            tr!(
                "versions differ: daemon {v}, akmctl {mine}, package {p}: \
restart the daemon after an upgrade (systemctl --user restart apple-kb-monitord.service)",
                v = v,
                mine = mine,
                p = p
            ),
        )
        .msg("versions.differ", vec![v.into(), mine.into(), p.into()]),
        Some(v) => check(
            "versions",
            Level::Ok,
            "ok",
            tr!("daemon {v} = akmctl, package {p}", v = v, p = p),
        )
        .msg("versions.same", vec![v.into(), p.into()]),
        None => check(
            "versions",
            Level::Info,
            "unknown",
            tr!("daemon version unknown (old interface)"),
        )
        .msg("versions.unknown", vec![]),
    }
}

fn stale_check(s: &akm_core::stale::Stale) -> Check {
    use akm_core::stale::Stale;
    let key = match s {
        Stale::DeletedExe { name, .. } => format!("deleted-{name}"),
        Stale::PlasmaOlder { .. } => "plasmashell-older".to_string(),
    };
    check(
        "versions",
        Level::Warn,
        key,
        format!("{}: {}", s.text(), s.fix()),
    )
    .msg(s.id(), [s.args(), vec![s.fix().to_string()]].concat())
}

fn check(id: &'static str, level: Level, key: impl Into<String>, text: impl Into<String>) -> Check {
    Check {
        id,
        level,
        text: text.into(),
        key: format!("{id}:{}", key.into()),
        msg: "",
        args: Vec::new(),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whole seconds of a Unix time read back from JSON.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // `as` saturates: negative or NaN gives 0
fn secs_of(t: f64) -> u64 {
    t as u64
}

fn now_s() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn state_dir() -> PathBuf {
    akm_core::history::default_path()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn alias_check(
    memory: &akm_core::alias::AliasMemory,
    mac: &str,
    actual: Option<&str>,
) -> Option<Check> {
    use akm_core::alias::{drift, AliasDrift};
    match drift(memory, mac, actual) {
        AliasDrift::NotRemembered | AliasDrift::Unknown => None,
        AliasDrift::Same => Some(check("alias", Level::Ok, "ok", tr!("keyboard alias as last set through this monitor")).msg("alias.same", vec![])),
        AliasDrift::Differs { expected, actual } => Some(check(
            "alias",
            Level::Info,
            format!("{}->{}", expected.alias, actual),
            tr!("keyboard alias is {actual}, last set to {alias} by {by} at {set_at} (unix): changed elsewhere (KDE Bluetooth settings, \
                 bluetoothctl) or the pairing was removed (akmctl repair, Plasma Forget); see the daemon journal for \
                 \"BlueZ alias\" lines", actual = format!("{:?}", actual), alias = format!("{:?}", expected.alias), by = expected.by, set_at = expected.set_at),
        ).msg("alias.differs", vec![format!("{actual:?}"), format!("{:?}", expected.alias), expected.by.clone(), expected.set_at.to_string()])),
    }
}

pub fn coredump_is_crash(si_code: Option<i64>) -> bool {
    si_code != Some(0)
}

pub fn exe_dirs() -> Vec<String> {
    std::env::var("AKM_SELFTEST_EXE_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(
            || vec!["/usr/bin".into(), "/usr/lib/apple-kb-monitor".into()],
            |v| {
                v.split(':')
                    .map(|d| d.trim_end_matches('/').to_string())
                    .collect()
            },
        )
}

pub fn crashes_from_journal(lines: &str, dirs: &[String]) -> Vec<(String, String, i64)> {
    lines
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            let exe = v["COREDUMP_EXE"].as_str()?.to_string();
            let (dir, base) = exe.rsplit_once('/')?;
            let base = base.to_string();
            if !OUR_EXES.contains(&base.as_str()) || !dirs.iter().any(|d| d == dir) {
                return None;
            }
            let code = v["COREDUMP_CODE"].as_str().and_then(|c| c.parse().ok());
            if !coredump_is_crash(code) {
                return None;
            }
            let sig = v["COREDUMP_SIGNAL_NAME"]
                .as_str()
                .or(v["COREDUMP_SIGNAL"].as_str())
                .unwrap_or("?")
                .to_string();
            let pid = v["COREDUMP_PID"]
                .as_str()
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
            Some((base, sig, pid))
        })
        .collect()
}

pub fn disk_level(avail: u64, total: u64) -> Level {
    if total == 0 {
        return Level::Info;
    }
    #[allow(clippy::cast_precision_loss)] // a ratio: 53 bits are plenty
    let pct = avail as f64 * 100.0 / total as f64;
    if avail < 100 << 20 || pct < 1.0 {
        Level::Bad
    } else if avail < 1 << 30 || pct < 5.0 {
        Level::Warn
    } else {
        Level::Ok
    }
}

pub fn new_grave(checks: &[Check], previous: &Value) -> Vec<Check> {
    let before: Vec<&str> = previous["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|c| Level::parse(c["level"].as_str().unwrap_or("")) == Level::Bad)
                .filter_map(|c| c["key"].as_str())
                .collect()
        })
        .unwrap_or_default();
    checks
        .iter()
        .filter(|c| c.level == Level::Bad && !before.contains(&c.key.as_str()))
        .cloned()
        .collect()
}

fn daemon_query() -> Result<(akm_core::Snapshot, Option<String>, Duration), String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let r = bus::connect().map_err(|e| e.to_string()).and_then(|c| {
            let s = bus::get_state(&c).map_err(|e| e.to_string())?;
            let v = bus::proxy(&c)
                .ok()
                .and_then(|p| p.get_property::<String>("DaemonVersion").ok());
            Ok((s, v))
        });
        let _ = tx.send(r.map(|(s, v)| (s, v, t0.elapsed())));
    });
    rx.recv_timeout(DBUS_TIMEOUT).unwrap_or_else(|_| {
        Err(tr!(
            "no answer within {} s (daemon stuck?)",
            DBUS_TIMEOUT.as_secs()
        ))
    })
}

#[allow(clippy::too_many_lines)] // flat list of independent checks
fn daemon_checks(
    out: &mut Vec<Check>,
    prev: &Value,
    now: f64,
    state: &mut BTreeMap<String, Value>,
) {
    let unit = run(
        "systemctl",
        &[
            "--user",
            "show",
            "apple-kb-monitord.service",
            "-p",
            "ActiveState,NRestarts,MainPID,UnitFileState",
        ],
    )
    .unwrap_or_default();
    let prop = |k: &str| {
        unit.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or("")
            .to_string()
    };
    let restarts: u64 = prop("NRestarts").parse().unwrap_or(0);
    let main_pid = prop("MainPID");
    let prev_restarts = prev["state"]["daemon_restarts"].as_u64();
    let prev_pid = prev["state"]["daemon_pid"]
        .as_str()
        .unwrap_or("")
        .to_string();
    state.insert("daemon_restarts".into(), json!(restarts));
    state.insert("daemon_pid".into(), json!(main_pid));
    if let Some(p) = prev_restarts.filter(|p| restarts > *p) {
        out.push(
            check(
                "daemon-restarts",
                Level::Bad,
                format!("{restarts}"),
                trn!(
                "apple-kb-monitord restarted {} time by systemd since the last pass (crash loop?)",
                "apple-kb-monitord restarted {} times by systemd since the last pass (crash loop?)",
                restarts - p,
                restarts - p
            ),
            )
            .msg("daemon-restarts.count", vec![(restarts - p).to_string()]),
        );
    } else if !prev_pid.is_empty() && prev_pid != "0" && main_pid != "0" && prev_pid != main_pid {
        out.push(
            check(
                "daemon-restarts",
                Level::Info,
                "pid",
                tr!(
                    "daemon pid changed {prev_pid} -> {main_pid} (restart)",
                    prev_pid = prev_pid,
                    main_pid = main_pid
                ),
            )
            .msg(
                "daemon-restarts.pid",
                vec![prev_pid.clone(), main_pid.clone()],
            ),
        );
    }
    match daemon_query() {
        Ok((s, version, took)) => {
            let ms = took.as_millis();
            out.push(if took > Duration::from_secs(1) {
                check(
                    "daemon",
                    Level::Warn,
                    "slow",
                    tr!("daemon answered GetState in {ms} ms (> 1 s)", ms = ms),
                )
                .msg("daemon.slow", vec![ms.to_string()])
            } else {
                check(
                    "daemon",
                    Level::Ok,
                    "ok",
                    tr!("daemon on the bus, GetState in {ms} ms", ms = ms),
                )
                .msg("daemon.ok", vec![ms.to_string()])
            });
            #[allow(clippy::cast_precision_loss)] // Unix seconds fit in 53 bits
            let age = now - s.last_update as f64;
            out.push(match (s.connected, s.last_update) {
                (true, 0) => check(
                    "freshness",
                    Level::Warn,
                    "never",
                    tr!("keyboard connected but never acquired"),
                )
                .msg("freshness.never", vec![]),
                (true, _) if age > 24.0 * 3600.0 => check(
                    "freshness",
                    Level::Bad,
                    "stale-24h",
                    tr!(
                        "keyboard connected, last acquisition {hours} h ago",
                        hours = format!("{:.1}", age / 3600.0)
                    ),
                )
                .msg("freshness.stale", vec![format!("{:.1}", age / 3600.0)]),
                (true, _) if age > 6.0 * 3600.0 => check(
                    "freshness",
                    Level::Warn,
                    "stale-6h",
                    tr!(
                        "keyboard connected, last acquisition {hours} h ago",
                        hours = format!("{:.1}", age / 3600.0)
                    ),
                )
                .msg("freshness.stale", vec![format!("{:.1}", age / 3600.0)]),
                (true, _) => check(
                    "freshness",
                    Level::Ok,
                    "ok",
                    tr!(
                        "last acquisition {minutes} min ago",
                        minutes = format!("{:.0}", age / 60.0)
                    ),
                )
                .msg("freshness.ok", vec![format!("{:.0}", age / 60.0)]),
                (false, _) => check(
                    "freshness",
                    Level::Info,
                    "disconnected",
                    tr!("keyboard not connected (nothing to acquire)"),
                )
                .msg("freshness.disconnected", vec![]),
            });
            if let Some(mac) = s.mac() {
                let actual = s.keyboard.as_ref().and_then(|k| k.device.alias.as_deref());
                let memory = akm_core::alias::AliasMemory::load(
                    &akm_core::alias::AliasMemory::default_path(),
                );
                out.extend(alias_check(&memory, mac, actual));
            }
            if let Some(e) = s.last_error.as_deref().filter(|e| !e.is_empty()) {
                out.push(
                    check(
                        "daemon-error",
                        Level::Warn,
                        short_key(e),
                        tr!("daemon last_error: {e}", e = e),
                    )
                    .msg("daemon-error", vec![e.to_string()]),
                );
            }
            let pkg = run("pacman", &["-Q", "apple-kb-monitor"])
                .and_then(|s| s.split_whitespace().nth(1).map(str::to_string));
            out.push(versions_check(
                version.as_deref(),
                akm_core::PKG_VERSION,
                pkg.as_deref(),
            ));
            out.extend(akm_core::stale::scan().iter().map(stale_check));
        }
        Err(e) => {
            let active = prop("ActiveState");
            out.push(
                check(
                    "daemon",
                    Level::Bad,
                    "unreachable",
                    tr!(
                        "daemon unreachable ({e}); unit state: {state}",
                        e = e,
                        state = if active.is_empty() {
                            "unknown"
                        } else {
                            &active
                        }
                    ),
                )
                .msg(
                    "daemon.unreachable",
                    vec![
                        e.clone(),
                        if active.is_empty() {
                            "unknown".into()
                        } else {
                            active.clone()
                        },
                    ],
                ),
            );
        }
    }
}

fn short_key(s: &str) -> String {
    s.chars().filter(|c| !c.is_ascii_digit()).take(60).collect()
}

#[allow(clippy::too_many_lines)] // flat list of independent checks
fn journal_checks(out: &mut Vec<Check>, since: u64) {
    let since_arg = format!("@{since}");
    match run(
        "journalctl",
        &[
            "--user",
            "-u",
            "apple-kb-monitord",
            "-p",
            "warning",
            "--since",
            &since_arg,
            "-o",
            "cat",
            "-q",
            "--no-pager",
            "-n",
            "500",
        ],
    ) {
        Some(t) => {
            let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.is_empty() {
                out.push(
                    check(
                        "journal-daemon",
                        Level::Ok,
                        "ok",
                        tr!("no daemon warning since the last pass"),
                    )
                    .msg("journal-daemon.ok", vec![]),
                );
            } else {
                let panic = lines.iter().any(|l| l.contains("panicked at"));
                out.push(
                    check(
                        "journal-daemon",
                        if panic { Level::Bad } else { Level::Warn },
                        if panic {
                            "panic".to_string()
                        } else {
                            short_key(lines[lines.len() - 1])
                        },
                        trn!(
                            "{} daemon warning/error line since the last pass, last: {}",
                            "{} daemon warning/error lines since the last pass, last: {}",
                            lines.len(),
                            lines.len(),
                            lines[lines.len() - 1].chars().take(160).collect::<String>()
                        ),
                    )
                    .msg(
                        "journal-daemon.lines",
                        vec![
                            lines.len().to_string(),
                            lines[lines.len() - 1].chars().take(160).collect(),
                        ],
                    ),
                );
            }
        }
        None => out.push(
            check(
                "journal-daemon",
                Level::Info,
                "unreadable",
                tr!("user journal not readable"),
            )
            .msg("journal-daemon.unreadable", vec![]),
        ),
    }
    if let Some(t) = run(
        "journalctl",
        &[
            "--user",
            "--since",
            &since_arg,
            "-g",
            "panicked at",
            "-o",
            "short",
            "-q",
            "--no-pager",
            "-n",
            "50",
        ],
    ) {
        let ours: Vec<&str> = t
            .lines()
            .filter(|l| OUR_EXES.iter().any(|e| l.contains(e)))
            .collect();
        if let Some(l) = ours.last() {
            out.push(check(
                "panic",
                Level::Bad,
                short_key(l.split("panicked at").nth(1).unwrap_or(l)),
                format!("panic: {}", l.chars().take(200).collect::<String>()),
            ));
        }
    }
    match run(
        "journalctl",
        &[
            "-u",
            "bluetooth",
            "-p",
            "err",
            "--since",
            &since_arg,
            "-o",
            "cat",
            "-q",
            "--no-pager",
            "-n",
            "200",
        ],
    ) {
        Some(t) => {
            let n = t.lines().filter(|l| !l.trim().is_empty()).count();
            out.push(if n == 0 {
                check(
                    "journal-bluetooth",
                    Level::Ok,
                    "ok",
                    tr!("no bluetoothd error since the last pass"),
                )
                .msg("journal-bluetooth.ok", vec![])
            } else {
                check(
                    "journal-bluetooth",
                    Level::Warn,
                    "errors",
                    trn!(
                        "{n} bluetoothd error line since the last pass",
                        "{n} bluetoothd error lines since the last pass",
                        n,
                        n = n
                    ),
                )
                .msg("journal-bluetooth.errors", vec![n.to_string()])
            });
        }
        None => out.push(
            check(
                "journal-bluetooth",
                Level::Info,
                "unreadable",
                tr!("system journal not readable (group systemd-journal)"),
            )
            .msg("journal-bluetooth.unreadable", vec![]),
        ),
    }
}

fn coredump_checks(out: &mut Vec<Check>, since: u64) {
    let since_arg = format!("@{since}");
    let fields = "COREDUMP_EXE,COREDUMP_SIGNAL,COREDUMP_SIGNAL_NAME,COREDUMP_CODE,COREDUMP_PID";
    match run(
        "journalctl",
        &[
            &format!("MESSAGE_ID={COREDUMP_MSGID}"),
            "--since",
            &since_arg,
            "-o",
            "json",
            &format!("--output-fields={fields}"),
            "-q",
            "--no-pager",
        ],
    ) {
        Some(t) => {
            let crashes = crashes_from_journal(&t, &exe_dirs());
            if crashes.is_empty() {
                out.push(
                    check(
                        "coredumps",
                        Level::Ok,
                        "ok",
                        tr!("no crash of our programs since the last pass"),
                    )
                    .msg("coredumps.ok", vec![]),
                );
            }
            for (exe, sig, pid) in crashes {
                out.push(
                    check(
                        "coredumps",
                        Level::Bad,
                        format!("{exe}:{sig}:{pid}"),
                        tr!(
                            "{exe} crashed ({sig}, pid {pid}): coredumpctl info {pid}",
                            exe = exe,
                            sig = sig,
                            pid = pid
                        ),
                    )
                    .msg(
                        "coredumps.crash",
                        vec![exe.clone(), sig.clone(), pid.to_string()],
                    ),
                );
            }
        }
        None => out.push(
            check(
                "coredumps",
                Level::Info,
                "unreadable",
                tr!("coredump journal not readable"),
            )
            .msg("coredumps.unreadable", vec![]),
        ),
    }
}

// statvfs field types vary per target (u32 or i32 on 32-bit): the casts are needed there.
#[allow(clippy::unnecessary_cast)]
fn statvfs(p: &Path) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &raw mut s) } == 0).then(|| {
        (
            s.f_bavail as u64 * s.f_frsize as u64,
            s.f_blocks as u64 * s.f_frsize as u64,
        )
    })
}

fn disk_checks(out: &mut Vec<Check>) {
    let state = state_dir();
    for (name, p) in [
        ("bluetooth", PathBuf::from("/var/lib/bluetooth")),
        ("state", state.clone()),
    ] {
        let probe = if p.exists() {
            p.clone()
        } else {
            p.parent().map(Path::to_path_buf).unwrap_or(p.clone())
        };
        match statvfs(&probe) {
            Some((avail, total)) => {
                let lvl = disk_level(avail, total);
                out.push(
                    check(
                        "disk",
                        lvl,
                        format!("{name}:{}", lvl.as_str()),
                        tr!(
                            "{}: {} MiB free of {} MiB",
                            p.display(),
                            avail >> 20,
                            total >> 20
                        ),
                    )
                    .msg(
                        "disk.free",
                        vec![
                            p.display().to_string(),
                            (avail >> 20).to_string(),
                            (total >> 20).to_string(),
                        ],
                    ),
                );
            }
            None => out.push(
                check(
                    "disk",
                    Level::Info,
                    format!("{name}:unknown"),
                    tr!("{}: statvfs failed", p.display()),
                )
                .msg("disk.unknown", vec![p.display().to_string()]),
            ),
        }
    }
}

fn history_check(out: &mut Vec<Check>) {
    let hist = akm_core::history::default_path();
    match std::fs::read_to_string(&hist) {
        Ok(t) => {
            let n = t.lines().count();
            let tail: Vec<&str> = t.lines().rev().take(1000).collect();
            let bad = tail
                .iter()
                .filter(|l| serde_json::from_str::<Value>(l).is_err())
                .count();
            let size = t.len() as u64;
            let lvl = if bad > 0 || size > 50 << 20 {
                Level::Warn
            } else {
                Level::Ok
            };
            out.push(
                check(
                    "history",
                    lvl,
                    if bad > 0 {
                        "corrupt"
                    } else if size > 50 << 20 {
                        "large"
                    } else {
                        "ok"
                    },
                    trn!(
                        "{hist}: {lines}, {kib} KiB, {bad} unreadable line in the last 1000",
                        "{hist}: {lines}, {kib} KiB, {bad} unreadable lines in the last 1000",
                        bad,
                        hist = hist.display(),
                        lines = trn!("{n} line", "{n} lines", n, n = n),
                        kib = size >> 10,
                        bad = bad
                    ),
                )
                .msg(
                    "history.stats",
                    vec![
                        bad.to_string(),
                        hist.display().to_string(),
                        n.to_string(),
                        (size >> 10).to_string(),
                    ],
                ),
            );
        }
        Err(_) if !hist.exists() => out.push(
            check("history", Level::Info, "absent", tr!("no history yet"))
                .msg("history.absent", vec![]),
        ),
        Err(e) => out.push(check(
            "history",
            Level::Warn,
            "unreadable",
            format!("{}: {e}", hist.display()),
        )),
    }
}

fn link_checks(out: &mut Vec<Check>) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = doctor::gather(None);
        let lvl = match r.verdict.0 {
            doctor::Level::Bad => Level::Bad,
            doctor::Level::Warn => Level::Warn,
            doctor::Level::Info => Level::Info,
            doctor::Level::Ok => Level::Ok,
        };
        let vid = doctor::verdict_id(&r.verdict.1);
        let mut verdict = vec![r.verdict.1.clone(), vid.to_string()];
        if vid == "link-up-fix" {
            verdict.extend(
                doctor::verdict_topics(&r.findings)
                    .iter()
                    .map(ToString::to_string),
            );
        }
        let _ = tx.send((
            lvl,
            r.health.unwrap_or_else(|| "?".into()),
            r.verdict.1,
            verdict,
        ));
    });
    match rx.recv_timeout(DOCTOR_TIMEOUT) {
        Ok((lvl, health, text, verdict)) => out.push(
            check(
                "link",
                lvl,
                health,
                tr!("akmctl doctor: {text}", text = text),
            )
            .msg("link.doctor", verdict),
        ),
        Err(_) => out.push(
            check(
                "link",
                Level::Bad,
                "doctor-stuck",
                tr!(
                "akmctl doctor did not finish within {} s (BlueZ or the daemon does not answer)",
                DOCTOR_TIMEOUT.as_secs()
            ),
            )
            .msg(
                "link.doctor-stuck",
                vec![DOCTOR_TIMEOUT.as_secs().to_string()],
            ),
        ),
    }
}

fn notify(c: &Check) {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return;
    };
    let hints: std::collections::HashMap<&str, zbus::zvariant::Value> =
        [("urgency", zbus::zvariant::Value::U8(2))]
            .into_iter()
            .collect();
    let _ = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "Apple Keyboard Monitor",
            0u32,
            "dialog-warning",
            tr!("Apple keyboard monitor: problem detected"),
            format!("{}\n(akmctl selftest)", c.text),
            Vec::<&str>::new(),
            hints,
            -1i32,
        ),
    );
}

/// Open issues read per page while looking for the one of a check.
const SEARCH_PAGE: usize = 50;

/// The open issue of this check, by the tag at the start of its title.
fn tagged<'a>(issues: &'a [Value], tag: &str) -> Option<&'a Value> {
    issues
        .iter()
        .find(|i| i["title"].as_str().is_some_and(|t| t.starts_with(tag)))
}

/// One issue per problem key, never twice.
fn gitea_issue(c: &Check) -> Result<String, String> {
    let api = std::env::var("AKM_GITEA_API").map_err(|_| "AKM_GITEA_API not set")?;
    let repo = std::env::var("AKM_GITEA_REPO").map_err(|_| "AKM_GITEA_REPO not set")?;
    let token_file = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("no HOME")?
        .join(".config/gitea/token");
    let token = std::fs::read_to_string(&token_file)
        .map_err(|e| format!("{}: {e}", token_file.display()))?;
    let dir = akm_core::paths::runtime_home();
    let hdr = dir.join(format!("akm-selftest-{}.hdr", std::process::id()));
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&hdr)
            .map_err(|e| e.to_string())?;
        writeln!(
            f,
            "Authorization: token {}\nContent-Type: application/json",
            token.trim()
        )
        .map_err(|e| e.to_string())?;
    }
    let tag = format!("[selfcheck:{}]", c.key);
    let res = (|| {
        // Every page: an open issue past the first page must not get a duplicate.
        for page in 1.. {
            let search = Command::new("curl")
                .args(["-sf", "-m", "20", "-H", &format!("@{}", hdr.display())])
                .arg(format!(
                    "{api}/repos/{repo}/issues?state=open&type=issues&limit={SEARCH_PAGE}&page={page}&q=selfcheck"
                ))
                .output()
                .map_err(|e| format!("curl: {e}"))?;
            let found: Value = serde_json::from_slice(&search.stdout)
                .map_err(|_| format!("Gitea search failed (exit {:?})", search.status.code()))?;
            let list = found.as_array().ok_or("Gitea search: not a list")?;
            if let Some(n) = tagged(list, &tag) {
                return Ok(format!("already open: #{}", n["number"]));
            }
            if list.len() < SEARCH_PAGE {
                break;
            }
        }
        let body = json!({
            "title": format!("{tag} {}", c.text.chars().take(120).collect::<String>()),
            "body": format!("Detected automatically by `akmctl selftest` on {} at {}.\n\n- check: `{}`\n- level: {}\n- detail: {}\n\nResult file: `{}`. See docs/TESTING.md.",
                hostname(), now_secs(), c.id, c.level.as_str(), c.text, state_dir().join("selfcheck.json").display()),
        });
        let post = Command::new("curl")
            .args([
                "-sf",
                "-m",
                "20",
                "-X",
                "POST",
                "-H",
                &format!("@{}", hdr.display()),
                "-d",
                &body.to_string(),
            ])
            .arg(format!("{api}/repos/{repo}/issues"))
            .output()
            .map_err(|e| format!("curl: {e}"))?;
        let v: Value = serde_json::from_slice(&post.stdout)
            .map_err(|_| format!("Gitea refused (exit {:?})", post.status.code()))?;
        Ok(format!("opened #{}", v["number"]))
    })();
    let _ = std::fs::remove_file(&hdr);
    res
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn run_selftest(a: &SelftestArgs) -> u8 {
    let path = a
        .state_file
        .clone()
        .unwrap_or_else(|| state_dir().join("selfcheck.json"));
    let previous: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let now = now_s();
    let since = previous["finished"]
        .as_f64()
        .map_or(now_secs().saturating_sub(3600), secs_of);
    let mut checks = Vec::new();
    let mut st = BTreeMap::new();
    daemon_checks(&mut checks, &previous, now, &mut st);
    link_checks(&mut checks);
    journal_checks(&mut checks, since);
    coredump_checks(&mut checks, since);
    disk_checks(&mut checks);
    history_check(&mut checks);

    let fresh = new_grave(&checks, &previous);
    let mut reported = Vec::new();
    for c in &fresh {
        if a.notify {
            notify(c);
        }
        if a.gitea_issue {
            match gitea_issue(c) {
                Ok(m) => reported.push(json!({"key": c.key, "gitea": m})),
                Err(e) => reported.push(json!({"key": c.key, "gitea_error": e})),
            }
        }
    }
    let worst = checks.iter().map(|c| c.level).max().unwrap_or(Level::Ok);
    let result = json!({
        "schema": 1,
        "tool": format!("akmctl {}", akm_core::PKG_VERSION),
        "since": since,
        "finished": now_s(),
        "verdict": worst.as_str(),
        "new_grave": fresh.iter().map(|c| c.key.clone()).collect::<Vec<_>>(),
        "reported": reported,
        "checks": checks.iter().map(|c| json!({"id": c.id, "level": c.level.as_str(), "key": c.key, "text": c.text, "msg": c.msg, "args": c.args})).collect::<Vec<_>>(),
        "state": st,
    });
    if !a.no_save {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, format!("{result:#}\n"))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .is_err()
        {
            eprintln!("akmctl: {}", tr!("cannot write {}", path.display()));
        }
    }
    if a.json {
        println!("{result}");
    } else {
        for c in &checks {
            let tag = match c.level {
                Level::Ok => "[ ok ]",
                Level::Info => "[info]",
                Level::Warn => "[ !! ]",
                Level::Bad => "[ KO ]",
            };
            println!("{tag} {:<17} {}", c.id, c.text);
        }
        println!(
            "{}",
            tr!(
                "\nverdict: {}{}",
                worst.as_str(),
                if fresh.is_empty() {
                    String::new()
                } else {
                    trn!(
                        " ({} new grave problem)",
                        " ({} new grave problems)",
                        fresh.len(),
                        fresh.len()
                    )
                }
            )
        );
    }
    if worst == Level::Bad {
        EXIT_ERROR
    } else {
        EXIT_OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_issue_is_found_by_its_tag() {
        let page = vec![
            json!({"number": 3, "title": "[selfcheck:disk] Disk"}),
            json!({"number": 7, "title": "[selfcheck:link] Link lost"}),
        ];
        assert_eq!(tagged(&page, "[selfcheck:link]").unwrap()["number"], 7);
        assert!(tagged(&page, "[selfcheck:fw]").is_none());
        let quoted = vec![json!({"number": 9, "title": "Re: [selfcheck:link] Link lost"})];
        assert!(tagged(&quoted, "[selfcheck:link]").is_none());
    }

    #[test]
    fn versions_see_the_release() {
        let c = versions_check(Some("3.1.0-26"), "3.1.0-27", Some("3.1.0-27"));
        assert_eq!(c.level, Level::Warn, "{}", c.text);
        assert!(c
            .text
            .contains("systemctl --user restart apple-kb-monitord.service"));
        let c = versions_check(Some("3.1.0-27"), "3.1.0-27", Some("3.1.0-27"));
        assert_eq!(c.level, Level::Ok);
        assert_eq!(
            versions_check(Some("3.1.0"), "3.1.0-27", Some("3.1.0-27")).level,
            Level::Ok
        );
        assert_eq!(
            versions_check(Some("3.0.9"), "3.1.0", None).level,
            Level::Warn
        );
        assert_eq!(versions_check(None, "3.1.0", None).level, Level::Info);
    }

    #[test]
    fn checks_carry_a_message_id_and_its_values() {
        // the System Settings module translates from msg + args, never from the text.
        let c = versions_check(Some("3.1.0-26"), "3.1.0-27", Some("3.1.0-27"));
        assert_eq!(
            (c.msg, c.args),
            (
                "versions.differ",
                vec!["3.1.0-26".into(), "3.1.0-27".into(), "3.1.0-27".into()]
            )
        );
        let c = versions_check(None, "3.1.0", None);
        assert_eq!((c.msg, c.args.len()), ("versions.unknown", 0));
        let c = stale_check(&akm_core::stale::Stale::PlasmaOlder {
            pid: 7,
            started: 1,
            installed: 2,
        });
        assert_eq!(c.msg, "versions.plasma-older");
        assert_eq!(c.args.first().map(String::as_str), Some("7"));
    }

    #[test]
    fn stale_process_gives_the_command() {
        use akm_core::stale::Stale;
        let c = stale_check(&Stale::DeletedExe {
            pid: 42,
            name: "akmctl",
            path: "/usr/bin/akmctl",
            fix: "reinstall akmctl",
        });
        assert_eq!(c.id, "versions");
        assert_eq!(c.level, Level::Warn);
        assert!(
            c.text.contains("(pid 42)") && c.text.ends_with("reinstall akmctl"),
            "{}",
            c.text
        );
        let c = stale_check(&Stale::PlasmaOlder {
            pid: 7,
            started: 1,
            installed: 2,
        });
        assert!(
            c.text
                .ends_with("systemctl --user restart plasma-plasmashell.service"),
            "{}",
            c.text
        );
    }

    #[test]
    fn coredump_by_kill_is_not_a_crash_but_abort_and_segv_are() {
        assert!(!coredump_is_crash(Some(0))); // SI_USER: kill -SEGV
        assert!(coredump_is_crash(Some(-6))); // SI_TKILL: abort(), panic=abort
        assert!(coredump_is_crash(Some(1))); // SEGV_MAPERR
        assert!(coredump_is_crash(None));
    }

    #[test]
    fn only_our_programs_are_reported() {
        let j = concat!(
            r#"{"COREDUMP_EXE":"/usr/bin/akmctl","COREDUMP_CODE":"-6","COREDUMP_SIGNAL_NAME":"SIGABRT","COREDUMP_PID":"42"}"#,
            "\n",
            r#"{"COREDUMP_EXE":"/usr/bin/notify-send","COREDUMP_CODE":"-6","COREDUMP_SIGNAL_NAME":"SIGABRT","COREDUMP_PID":"43"}"#,
            "\n",
            r#"{"COREDUMP_EXE":"/usr/bin/apple-kb-monitord","COREDUMP_CODE":"0","COREDUMP_SIGNAL_NAME":"SIGSEGV","COREDUMP_PID":"44"}"#,
            "\nnot json\n"
        );
        assert_eq!(
            crashes_from_journal(j, &["/usr/bin".to_string()]),
            vec![("akmctl".to_string(), "SIGABRT".to_string(), 42)]
        );
    }

    #[test]
    fn alias_differing_from_the_remembered_one_is_reported_as_info() {
        let mut m = akm_core::alias::AliasMemory::default();
        let mac = "AA:BB:CC:DD:EE:F1";
        assert_eq!(
            alias_check(&m, mac, Some("x")),
            None,
            "never set here: nothing"
        );
        m.remember(mac, "Desk", ":1.7 pid 12 (akmctl)", 1000);
        assert_eq!(alias_check(&m, mac, None), None, "unknown alias: nothing");
        assert_eq!(alias_check(&m, mac, Some("Desk")).unwrap().level, Level::Ok);
        let c = alias_check(&m, mac, Some("Alice's keyboard")).unwrap();
        assert_eq!(c.level, Level::Info, "information, never a fault");
        assert!(
            c.text.contains("\"Desk\"") && c.text.contains("akmctl") && c.text.contains("1000"),
            "{}",
            c.text
        );
        assert_eq!(c.key, "alias:Desk->Alice's keyboard");
        assert_eq!(c.msg, "alias.differs");
        assert_eq!(
            c.args,
            [
                "\"Alice's keyboard\"",
                "\"Desk\"",
                ":1.7 pid 12 (akmctl)",
                "1000"
            ]
        );
        assert!(
            new_grave(&[c], &serde_json::Value::Null).is_empty(),
            "info is never notified"
        );
    }

    #[test]
    fn disk_thresholds() {
        let g = 1u64 << 30;
        assert_eq!(disk_level(50 * g, 100 * g), Level::Ok);
        assert_eq!(disk_level(3 * g, 100 * g), Level::Warn);
        assert_eq!(disk_level(50 << 20, 100 * g), Level::Bad);
        assert_eq!(disk_level(0, 0), Level::Info);
    }

    #[test]
    fn only_new_grave_problems_are_reported_once() {
        let c = |key: &str, level| Check {
            id: "x",
            level,
            text: String::new(),
            key: key.into(),
            msg: "",
            args: Vec::new(),
        };
        let now = vec![c("a", Level::Bad), c("b", Level::Bad), c("c", Level::Warn)];
        let prev = json!({"checks": [{"key": "a", "level": "bad"}, {"key": "b", "level": "warn"}]});
        let fresh = new_grave(&now, &prev);
        assert_eq!(
            fresh.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
            vec!["b"]
        );
        assert_eq!(new_grave(&now, &Value::Null).len(), 2);
    }
}
