//! `akmctl selftest`: periodic health check of the installed monitor, run by
//! `apple-kb-monitor-selfcheck.timer` every 15 minutes (docs/QA-AUTOMATIQUE.md).
//!
//! Never touches the keyboard: it only asks the daemon (D-Bus, with a
//! timeout), systemd, the journal, coredumpctl data, /proc and the file
//! system. The link diagnosis is `akmctl doctor` (read-only; its hidraw step
//! is an access check by `open()`, no report is exchanged).
//!
//! Results go to `$XDG_STATE_HOME/apple-kb-monitor/selfcheck.json`. A problem
//! is identified by a stable key: only a *new* grave problem (level `bad`,
//! absent from the previous pass) raises a desktop notification and,
//! optionally (`--gitea-issue`), one Gitea issue per key (never twice).

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

/// Programs whose crashes and panics concern this project.
pub const OUR_EXES: &[&str] = &[
    "apihub-app",
    "apple-kb-monitord",
    "rssi-helper",
    "akm-helper",
    "akmctl",
];
/// systemd-coredump journal entries.
const COREDUMP_MSGID: &str = "fc2e22bc6ee647b6b90729ab34a250b1";
/// The daemon must answer `GetState` within this delay (a slow daemon
/// freezes every client that calls it synchronously).
const DBUS_TIMEOUT: Duration = Duration::from_secs(5);
/// `akmctl doctor` (BlueZ + daemon + journal) must finish within this delay.
const DOCTOR_TIMEOUT: Duration = Duration::from_secs(20);
/// UI heartbeat older than this while the process lives = frozen window.
pub const HEARTBEAT_STALE_S: f64 = 10.0;
/// A frame longer than this is a visible stall.
pub const FRAME_WARN_MS: f64 = 250.0;
const GITEA_API: &str = "https://gitea.pika.agenceapi.fr/api/v1";
const GITEA_REPO: &str = "adminapi/apple-kb-monitor";

#[derive(Args, Debug, Clone)]
pub struct SelftestArgs {
    /// Print the result as JSON (the same object as selfcheck.json)
    #[arg(long)]
    pub json: bool,
    /// Desktop notification for each NEW grave problem (deduplicated)
    #[arg(long)]
    pub notify: bool,
    /// Also open one Gitea issue per new grave problem (deduplicated; needs
    /// ~/.config/gitea/token). Off by default
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
    /// Stable identity of the problem (deduplication of notifications/issues).
    pub key: String,
}

fn check(id: &'static str, level: Level, key: impl Into<String>, text: impl Into<String>) -> Check {
    Check {
        id,
        level,
        text: text.into(),
        key: format!("{id}:{}", key.into()),
    }
}

fn now_s() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn state_dir() -> PathBuf {
    akm_core::history::default_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── pure assessments (unit-tested) ─────────────────────────────────────────

/// The BlueZ alias against the one last set through this program
/// (`alias.json`). A difference is reported as INFO (not a fault of the
/// monitor): something else renamed the keyboard, or its pairing was removed
/// and made again (BlueZ forgets the alias with the device). `None` when no
/// alias was ever set through this program or the alias is unknown.
pub fn alias_check(
    memory: &akm_core::alias::AliasMemory,
    mac: &str,
    actual: Option<&str>,
) -> Option<Check> {
    use akm_core::alias::{drift, AliasDrift};
    match drift(memory, mac, actual) {
        AliasDrift::NotRemembered | AliasDrift::Unknown => None,
        AliasDrift::Same => Some(check("alias", Level::Ok, "ok", "keyboard alias as last set through this monitor")),
        AliasDrift::Differs { expected, actual } => Some(check(
            "alias",
            Level::Info,
            format!("{}->{}", expected.alias, actual),
            format!(
                "keyboard alias is {actual:?}, last set to {:?} by {} at {} (unix): changed elsewhere (KDE Bluetooth settings, \
                 bluetoothctl) or the pairing was removed (akmctl repair, Plasma Forget); see the daemon journal for \
                 \"BlueZ alias\" lines",
                expected.alias, expected.by, expected.set_at
            ),
        )),
    }
}

/// A core dump is a crash unless it was requested by `kill()` (si_code
/// SI_USER = 0). abort()/panic=abort is SI_TKILL (-6): a crash.
pub fn coredump_is_crash(si_code: Option<i64>) -> bool {
    si_code != Some(0)
}

/// Core dumps of our programs in journal JSON lines (`journalctl -o json`).
/// Directories of the installed programs. Builds run from a source tree
/// (target/, test sandboxes) are not the user's monitor and are ignored;
/// `AKM_SELFTEST_EXE_DIRS` (colon-separated) replaces the list.
pub fn exe_dirs() -> Vec<String> {
    std::env::var("AKM_SELFTEST_EXE_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.split(':')
                .map(|d| d.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_else(|| vec!["/usr/bin".into(), "/usr/lib/apple-kb-monitor".into()])
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

/// Heartbeat contract (docs/QA-AUTOMATIQUE.md §3.2): the UI rewrites
/// `$XDG_RUNTIME_DIR/apple-kb-monitor/ui-heartbeat.json` at least every 2 s.
pub fn assess_heartbeat(pid: u32, hb: Option<&Value>, now: f64) -> Check {
    let Some(hb) = hb else {
        return check(
            "ui-heartbeat",
            Level::Info,
            "absent",
            format!(
                "apihub-app (pid {pid}) runs without the heartbeat contract: freeze not measurable"
            ),
        );
    };
    if hb["pid"].as_u64() != Some(pid as u64) {
        return check(
            "ui-heartbeat",
            Level::Info,
            "other-pid",
            format!(
                "heartbeat belongs to pid {} (window pid {pid}): not started yet?",
                hb["pid"]
            ),
        );
    }
    let age = now - hb["ts_ms"].as_f64().unwrap_or(0.0) / 1000.0;
    if age > HEARTBEAT_STALE_S {
        return check(
            "ui-heartbeat",
            Level::Bad,
            "frozen",
            format!("apihub-app window frozen: no frame for {age:.0} s (pid {pid})"),
        );
    }
    let fmax = hb["frame_max_ms"].as_f64().unwrap_or(0.0);
    if fmax > FRAME_WARN_MS {
        return check(
            "ui-heartbeat",
            Level::Warn,
            "slow-frame",
            format!("apihub-app frame of {fmax:.0} ms (> {FRAME_WARN_MS:.0} ms): visible stall"),
        );
    }
    check(
        "ui-heartbeat",
        Level::Ok,
        "ok",
        format!("window responsive (last frame {age:.1} s ago, max {fmax:.0} ms)"),
    )
}

/// CPU share used by a process between two passes (1.0 = one core).
pub fn cpu_share(prev: Option<(f64, f64)>, ticks_s: f64, at: f64) -> Option<f64> {
    let (p_ticks, p_at) = prev?;
    let dt = at - p_at;
    (dt > 30.0).then(|| (ticks_s - p_ticks).max(0.0) / dt)
}

/// Disk space verdict for a mount point.
pub fn disk_level(avail: u64, total: u64) -> Level {
    if total == 0 {
        return Level::Info;
    }
    let pct = avail as f64 * 100.0 / total as f64;
    if avail < 100 << 20 || pct < 1.0 {
        Level::Bad
    } else if avail < 1 << 30 || pct < 5.0 {
        Level::Warn
    } else {
        Level::Ok
    }
}

/// Keys of grave problems that were not grave at the previous pass.
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

// ── gathering ──────────────────────────────────────────────────────────────

/// `GetState` + `DaemonVersion` in a worker thread: a daemon that does not
/// answer is reported instead of hanging the self-test.
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
        Err(format!(
            "no answer within {} s (daemon stuck?)",
            DBUS_TIMEOUT.as_secs()
        ))
    })
}

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
        out.push(check(
            "daemon-restarts",
            Level::Bad,
            format!("{restarts}"),
            format!("apple-kb-monitord restarted {} time(s) by systemd since the last pass (crash loop?)", restarts - p),
        ));
    } else if !prev_pid.is_empty() && prev_pid != "0" && main_pid != "0" && prev_pid != main_pid {
        out.push(check(
            "daemon-restarts",
            Level::Info,
            "pid",
            format!("daemon pid changed {prev_pid} -> {main_pid} (restart)"),
        ));
    }
    match daemon_query() {
        Ok((s, version, took)) => {
            let ms = took.as_millis();
            out.push(if took > Duration::from_secs(1) {
                check(
                    "daemon",
                    Level::Warn,
                    "slow",
                    format!("daemon answered GetState in {ms} ms (> 1 s)"),
                )
            } else {
                check(
                    "daemon",
                    Level::Ok,
                    "ok",
                    format!("daemon on the bus, GetState in {ms} ms"),
                )
            });
            let age = now - s.last_update as f64;
            out.push(match (s.connected, s.last_update) {
                (true, 0) => check(
                    "freshness",
                    Level::Warn,
                    "never",
                    "keyboard connected but never acquired",
                ),
                (true, _) if age > 24.0 * 3600.0 => check(
                    "freshness",
                    Level::Bad,
                    "stale-24h",
                    format!(
                        "keyboard connected, last acquisition {:.1} h ago",
                        age / 3600.0
                    ),
                ),
                (true, _) if age > 6.0 * 3600.0 => check(
                    "freshness",
                    Level::Warn,
                    "stale-6h",
                    format!(
                        "keyboard connected, last acquisition {:.1} h ago",
                        age / 3600.0
                    ),
                ),
                (true, _) => check(
                    "freshness",
                    Level::Ok,
                    "ok",
                    format!("last acquisition {:.0} min ago", age / 60.0),
                ),
                (false, _) => check(
                    "freshness",
                    Level::Info,
                    "disconnected",
                    "keyboard not connected (nothing to acquire)",
                ),
            });
            if let Some(mac) = s.mac() {
                let actual = s.keyboard.as_ref().and_then(|k| k.device.alias.as_deref());
                let memory = akm_core::alias::AliasMemory::load(
                    &akm_core::alias::AliasMemory::default_path(),
                );
                out.extend(alias_check(&memory, mac, actual));
            }
            if let Some(e) = s.last_error.as_deref().filter(|e| !e.is_empty()) {
                out.push(check(
                    "daemon-error",
                    Level::Warn,
                    short_key(e),
                    format!("daemon last_error: {e}"),
                ));
            }
            let mine = env!("CARGO_PKG_VERSION");
            let pkg = run("pacman", &["-Q", "apple-kb-monitor"])
                .and_then(|s| s.split_whitespace().nth(1).map(str::to_string));
            let pkg_v = pkg.as_deref().map(|p| p.split('-').next().unwrap_or(p));
            match version.as_deref() {
                Some(v) if v != mine || pkg_v.is_some_and(|p| p != v) => out.push(check(
                    "versions",
                    Level::Warn,
                    format!("{v}/{mine}/{}", pkg.as_deref().unwrap_or("-")),
                    format!(
                        "versions differ: daemon {v}, akmctl {mine}, package {} (restart the daemon after an upgrade)",
                        pkg.as_deref().unwrap_or("?")
                    ),
                )),
                Some(v) => out.push(check("versions", Level::Ok, "ok", format!("daemon {v} = akmctl, package {}", pkg.as_deref().unwrap_or("?")))),
                None => out.push(check("versions", Level::Info, "unknown", "daemon version unknown (old interface)")),
            }
        }
        Err(e) => {
            let active = prop("ActiveState");
            out.push(check(
                "daemon",
                Level::Bad,
                "unreachable",
                format!(
                    "daemon unreachable ({e}); unit state: {}",
                    if active.is_empty() {
                        "unknown"
                    } else {
                        &active
                    }
                ),
            ));
        }
    }
}

fn short_key(s: &str) -> String {
    // Digits vary (counts, ages): keep the shape of the message.
    s.chars().filter(|c| !c.is_ascii_digit()).take(60).collect()
}

fn journal_checks(out: &mut Vec<Check>, since: u64) {
    let since_arg = format!("@{since}");
    // Warnings and errors of the daemon since the previous pass.
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
                out.push(check(
                    "journal-daemon",
                    Level::Ok,
                    "ok",
                    "no daemon warning since the last pass",
                ));
            } else {
                let panic = lines.iter().any(|l| l.contains("panicked at"));
                out.push(check(
                    "journal-daemon",
                    if panic { Level::Bad } else { Level::Warn },
                    if panic {
                        "panic".to_string()
                    } else {
                        short_key(lines[lines.len() - 1])
                    },
                    format!(
                        "{} daemon warning/error line(s) since the last pass, last: {}",
                        lines.len(),
                        lines[lines.len() - 1].chars().take(160).collect::<String>()
                    ),
                ));
            }
        }
        None => out.push(check(
            "journal-daemon",
            Level::Info,
            "unreadable",
            "user journal not readable",
        )),
    }
    // Panics of any of our programs in the user journal (the window logs to it
    // when started from the desktop).
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
    // bluetoothd errors (system journal; group systemd-journal or wheel needed).
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
                    "no bluetoothd error since the last pass",
                )
            } else {
                check(
                    "journal-bluetooth",
                    Level::Warn,
                    "errors",
                    format!("{n} bluetoothd error line(s) since the last pass"),
                )
            });
        }
        None => out.push(check(
            "journal-bluetooth",
            Level::Info,
            "unreadable",
            "system journal not readable (group systemd-journal)",
        )),
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
                out.push(check(
                    "coredumps",
                    Level::Ok,
                    "ok",
                    "no crash of our programs since the last pass",
                ));
            }
            for (exe, sig, pid) in crashes {
                out.push(check(
                    "coredumps",
                    Level::Bad,
                    format!("{exe}:{sig}:{pid}"),
                    format!("{exe} crashed ({sig}, pid {pid}): coredumpctl info {pid}"),
                ));
            }
        }
        None => out.push(check(
            "coredumps",
            Level::Info,
            "unreadable",
            "coredump journal not readable",
        )),
    }
}

/// apihub-app processes of this user: (pid, utime+stime in seconds).
fn ui_processes() -> Vec<(u32, f64)> {
    let uid = unsafe { libc::getuid() };
    let tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let mut v = Vec::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(comm) = std::fs::read_to_string(e.path().join("comm")) else {
            continue;
        };
        if comm.trim() != "apihub-app" {
            continue;
        }
        let owner = std::fs::read_to_string(e.path().join("status"))
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("Uid:")
                        .and_then(|u| u.split_whitespace().next()?.parse::<u32>().ok())
                })
            });
        if owner != Some(uid) {
            continue;
        }
        let stat = std::fs::read_to_string(e.path().join("stat")).unwrap_or_default();
        let after = stat.rsplit_once(')').map(|(_, a)| a).unwrap_or("");
        let f: Vec<&str> = after.split_whitespace().collect();
        let ticks = f.get(11).and_then(|u| u.parse::<f64>().ok()).unwrap_or(0.0)
            + f.get(12).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        v.push((pid, ticks / tck));
    }
    v
}

pub fn heartbeat_path() -> PathBuf {
    let run = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    run.join("apple-kb-monitor/ui-heartbeat.json")
}

fn ui_checks(out: &mut Vec<Check>, prev: &Value, now: f64, state: &mut BTreeMap<String, Value>) {
    let procs = ui_processes();
    if procs.is_empty() {
        out.push(check("ui", Level::Ok, "none", "no apihub-app window open"));
        return;
    }
    let hb: Option<Value> = std::fs::read_to_string(heartbeat_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let mut cpu = serde_json::Map::new();
    for (pid, secs) in procs {
        out.push(assess_heartbeat(pid, hb.as_ref(), now));
        let p = &prev["state"]["ui_cpu"][pid.to_string()];
        let prev_sample = p[0].as_f64().zip(p[1].as_f64());
        if let Some(share) = cpu_share(prev_sample, secs, now) {
            if share > 0.9 {
                out.push(check("ui-cpu", Level::Warn, "busy", format!("apihub-app (pid {pid}) used {:.0} % of a core since the last pass (busy loop?)", share * 100.0)));
            }
        }
        cpu.insert(pid.to_string(), json!([secs, now]));
    }
    state.insert("ui_cpu".into(), Value::Object(cpu));
}

fn statvfs(p: &Path) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| {
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
                out.push(check(
                    "disk",
                    lvl,
                    format!("{name}:{}", lvl.as_str()),
                    format!(
                        "{}: {} MiB free of {} MiB",
                        p.display(),
                        avail >> 20,
                        total >> 20
                    ),
                ));
            }
            None => out.push(check(
                "disk",
                Level::Info,
                format!("{name}:unknown"),
                format!("{}: statvfs failed", p.display()),
            )),
        }
    }
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
            out.push(check(
                "history",
                lvl,
                if bad > 0 {
                    "corrupt"
                } else if size > 50 << 20 {
                    "large"
                } else {
                    "ok"
                },
                format!(
                    "{}: {n} lines, {} KiB, {bad} unreadable line(s) in the last 1000",
                    hist.display(),
                    size >> 10
                ),
            ));
        }
        Err(_) if !hist.exists() => {
            out.push(check("history", Level::Info, "absent", "no history yet"))
        }
        Err(e) => out.push(check(
            "history",
            Level::Warn,
            "unreadable",
            format!("{}: {e}", hist.display()),
        )),
    }
}

fn link_checks(out: &mut Vec<Check>) {
    // `doctor` asks BlueZ and the daemon without a reply timeout: a stuck
    // peer must not hang the self-test (it is reported instead).
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = doctor::gather(None);
        let lvl = match r.verdict.0 {
            doctor::Level::Bad => Level::Bad,
            doctor::Level::Warn => Level::Warn,
            doctor::Level::Info => Level::Info,
            doctor::Level::Ok => Level::Ok,
        };
        let _ = tx.send((lvl, r.health.unwrap_or_else(|| "?".into()), r.verdict.1));
    });
    match rx.recv_timeout(DOCTOR_TIMEOUT) {
        Ok((lvl, health, text)) => {
            out.push(check("link", lvl, health, format!("akmctl doctor: {text}")))
        }
        Err(_) => out.push(check(
            "link",
            Level::Bad,
            "doctor-stuck",
            format!(
                "akmctl doctor did not finish within {} s (BlueZ or the daemon does not answer)",
                DOCTOR_TIMEOUT.as_secs()
            ),
        )),
    }
}

// ── notification and issue ─────────────────────────────────────────────────

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
            "Apple keyboard monitor: problem detected",
            format!("{}\n(akmctl selftest)", c.text),
            Vec::<&str>::new(),
            hints,
            -1i32,
        ),
    );
}

/// One issue per problem key, never twice: the key is in the title and the
/// open issues are searched first. Uses curl; the token stays out of argv.
fn gitea_issue(c: &Check) -> Result<String, String> {
    let token_file = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("no HOME")?
        .join(".config/gitea/token");
    let token = std::fs::read_to_string(&token_file)
        .map_err(|e| format!("{}: {e}", token_file.display()))?;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
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
        let search = Command::new("curl")
            .args(["-sf", "-m", "20", "-H", &format!("@{}", hdr.display())])
            .arg(format!(
                "{GITEA_API}/repos/{GITEA_REPO}/issues?state=open&type=issues&limit=50&q=selfcheck"
            ))
            .output()
            .map_err(|e| format!("curl: {e}"))?;
        let found: Value = serde_json::from_slice(&search.stdout).unwrap_or(Value::Null);
        if let Some(n) = found.as_array().and_then(|a| {
            a.iter()
                .find(|i| i["title"].as_str().is_some_and(|t| t.contains(&tag)))
        }) {
            return Ok(format!("already open: #{}", n["number"]));
        }
        let body = json!({
            "title": format!("{tag} {}", c.text.chars().take(120).collect::<String>()),
            "body": format!("Detected automatically by `akmctl selftest` on {} at {}.\n\n- check: `{}`\n- level: {}\n- detail: {}\n\nResult file: `{}`. See docs/QA-AUTOMATIQUE.md.",
                hostname(), now_s() as u64, c.id, c.level.as_str(), c.text, state_dir().join("selfcheck.json").display()),
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
            .arg(format!("{GITEA_API}/repos/{GITEA_REPO}/issues"))
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

// ── entry point ────────────────────────────────────────────────────────────

pub fn run_selftest(a: SelftestArgs) -> u8 {
    let path = a
        .state_file
        .clone()
        .unwrap_or_else(|| state_dir().join("selfcheck.json"));
    let previous: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let now = now_s();
    // First pass: look back one hour.
    let since = previous["finished"]
        .as_f64()
        .map(|t| t as u64)
        .unwrap_or(now as u64 - 3600);
    let mut checks = Vec::new();
    let mut st = BTreeMap::new();
    daemon_checks(&mut checks, &previous, now, &mut st);
    link_checks(&mut checks);
    journal_checks(&mut checks, since);
    coredump_checks(&mut checks, since);
    ui_checks(&mut checks, &previous, now, &mut st);
    disk_checks(&mut checks);

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
        "tool": format!("akmctl {}", env!("CARGO_PKG_VERSION")),
        "since": since,
        "finished": now_s(),
        "verdict": worst.as_str(),
        "new_grave": fresh.iter().map(|c| c.key.clone()).collect::<Vec<_>>(),
        "reported": reported,
        "checks": checks.iter().map(|c| json!({"id": c.id, "level": c.level.as_str(), "key": c.key, "text": c.text})).collect::<Vec<_>>(),
        "state": st,
    });
    if !a.no_save {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, format!("{result:#}\n"))
            .and_then(|_| std::fs::rename(&tmp, &path))
            .is_err()
        {
            eprintln!("akmctl: cannot write {}", path.display());
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
            "\nverdict: {}{}",
            worst.as_str(),
            if fresh.is_empty() {
                String::new()
            } else {
                format!(" ({} new grave problem(s))", fresh.len())
            }
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
    fn coredump_by_kill_is_not_a_crash_but_abort_and_segv_are() {
        assert!(!coredump_is_crash(Some(0))); // SI_USER: kill -SEGV
        assert!(coredump_is_crash(Some(-6))); // SI_TKILL: abort(), panic=abort
        assert!(coredump_is_crash(Some(1))); // SEGV_MAPERR
        assert!(coredump_is_crash(None));
    }

    #[test]
    fn only_our_programs_are_reported() {
        let j = concat!(
            r#"{"COREDUMP_EXE":"/usr/bin/apihub-app","COREDUMP_CODE":"-6","COREDUMP_SIGNAL_NAME":"SIGABRT","COREDUMP_PID":"42"}"#,
            "\n",
            r#"{"COREDUMP_EXE":"/usr/bin/notify-send","COREDUMP_CODE":"-6","COREDUMP_SIGNAL_NAME":"SIGABRT","COREDUMP_PID":"43"}"#,
            "\n",
            r#"{"COREDUMP_EXE":"/usr/bin/apple-kb-monitord","COREDUMP_CODE":"0","COREDUMP_SIGNAL_NAME":"SIGSEGV","COREDUMP_PID":"44"}"#,
            "\nnot json\n"
        );
        assert_eq!(
            crashes_from_journal(j, &["/usr/bin".to_string()]),
            vec![("apihub-app".to_string(), "SIGABRT".to_string(), 42)]
        );
    }

    #[test]
    fn heartbeat_assessment() {
        let now = 1000.0;
        assert_eq!(assess_heartbeat(7, None, now).level, Level::Info);
        let hb = |pid: u32, ts: f64, fmax: f64| json!({"pid": pid, "ts_ms": ts * 1000.0, "frame_max_ms": fmax});
        assert_eq!(
            assess_heartbeat(7, Some(&hb(7, 999.0, 16.0)), now).level,
            Level::Ok
        );
        assert_eq!(
            assess_heartbeat(7, Some(&hb(7, 980.0, 16.0)), now).level,
            Level::Bad
        );
        assert_eq!(
            assess_heartbeat(7, Some(&hb(7, 999.0, 900.0)), now).level,
            Level::Warn
        );
        assert_eq!(
            assess_heartbeat(7, Some(&hb(8, 999.0, 16.0)), now).level,
            Level::Info
        );
    }

    #[test]
    fn cpu_share_needs_two_samples_far_enough_apart() {
        assert_eq!(cpu_share(None, 10.0, 100.0), None);
        assert_eq!(cpu_share(Some((0.0, 90.0)), 10.0, 100.0), None);
        assert_eq!(cpu_share(Some((0.0, 0.0)), 900.0, 900.0), Some(1.0));
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
        m.remember(mac, "Bureau", ":1.7 pid 12 (akmctl)", 1000);
        assert_eq!(alias_check(&m, mac, None), None, "unknown alias: nothing");
        assert_eq!(
            alias_check(&m, mac, Some("Bureau")).unwrap().level,
            Level::Ok
        );
        let c = alias_check(&m, mac, Some("Clavier de alice")).unwrap();
        assert_eq!(c.level, Level::Info, "information, never a fault");
        assert!(
            c.text.contains("\"Bureau\"") && c.text.contains("akmctl") && c.text.contains("1000"),
            "{}",
            c.text
        );
        assert_eq!(c.key, "alias:Bureau->Clavier de alice");
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
