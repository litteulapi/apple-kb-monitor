//! `akmctl` — command-line client of apple-kb-monitord.

mod bus;
mod cli;
mod doctor;
mod dump;
mod fnmode;
mod firmware;
mod histcmd;
mod info;
mod ledcmd;
mod migrate;
mod passive;
mod render;
mod repair;
mod selftest;
mod status;
mod when;

use std::io::Write;
use std::process::{Command as Proc, ExitCode};

use clap::{CommandFactory, Parser};
use cli::*;

const HELPER: &str = "/usr/lib/apple-kb-monitor/akm-helper";
/// Absolute path: never resolved through `$PATH` (the program runs as root).
const PKEXEC: &str = "/usr/bin/pkexec";

/// Program handed to pkexec. The path is a constant of the binary: no
/// environment variable can redirect it in a release build (#154). Only
/// the unit-test build honours `AKM_HELPER`.
#[cfg(not(test))]
fn helper_path() -> String {
    HELPER.to_string()
}

#[cfg(test)]
fn helper_path() -> String {
    std::env::var("AKM_HELPER").unwrap_or_else(|_| HELPER.to_string())
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let ok = !e.use_stderr();
            let _ = e.print();
            return ExitCode::from(if ok { EXIT_OK } else { EXIT_USAGE });
        }
    };
    ExitCode::from(run(cli.command))
}

fn run(cmd: Command) -> u8 {
    match cmd {
        Command::Status { json } => cmd_status(json),
        Command::Get { what: GetCmd::Fnmode } => match fnmode::read() {
            Ok(m) => {
                println!("{m}");
                EXIT_OK
            }
            Err(e) => fail(&e),
        },
        Command::Set { what: SetCmd::Fnmode { mode, persist } } => cmd_set_fnmode(mode, persist),
        Command::Rename { name, reset: _, mac } => cmd_rename(name.as_deref().unwrap_or(""), mac),
        Command::Watch => cmd_watch(),
        Command::History(h) => cmd_history(h),
        Command::Graph { span } => cmd_graph(span),
        Command::Waybar => cmd_waybar(),
        Command::Metrics => cmd_metrics(),
        Command::Led { name, state } => match ledcmd::run(name, state) {
            Ok(msg) => {
                println!("{msg}");
                EXIT_OK
            }
            Err(e) => fail(&e),
        },
        Command::Info { json } => cmd_info(json),
        Command::Firmware { json } => cmd_firmware(json),
        Command::Dump { json } => match dump::run() {
            Ok(d) => {
                if json {
                    println!("{}", dump::to_json(&d));
                } else {
                    print!("{}", dump::to_text(&d));
                }
                EXIT_OK
            }
            Err(e) => fail(&e),
        },
        Command::Doctor { json, mac } => {
            let r = doctor::gather(mac.as_deref());
            if json {
                println!("{}", doctor::to_json(&r));
            } else {
                print!("{}", doctor::to_text(&r));
            }
            if r.verdict.0 >= doctor::Level::Bad {
                EXIT_ERROR
            } else {
                EXIT_OK
            }
        }
        Command::Repair { mac, force } => repair::run(mac.as_deref(), force),
        Command::Selftest(a) => selftest::run_selftest(a),
        Command::Completions { shell } => {
            let mut c = Cli::command();
            clap_complete::generate(clap_complete::Shell::from(shell), &mut c, "akmctl", &mut std::io::stdout());
            EXIT_OK
        }
        Command::Man => {
            let mut buf = Vec::new();
            if clap_mangen::Man::new(Cli::command()).render(&mut buf).is_err() {
                return fail("cannot render the manual page");
            }
            let _ = std::io::stdout().write_all(&buf);
            EXIT_OK
        }
    }
}

fn fail(msg: &str) -> u8 {
    eprintln!("akmctl: {msg}");
    EXIT_ERROR
}

fn cmd_status(json: bool) -> u8 {
    let fm = fnmode::read().ok();
    let conn = bus::connect();
    let snap = conn.as_ref().map_err(|e| bus::BusError::Absent(e.to_string())).and_then(bus::get_state);
    match snap {
        Ok(s) => {
            if json {
                let mut v = status::to_json(&s, fm, None);
                // Passive listening state (Fn-lock, sleep, wake, Eject), null if unknown.
                if let Ok(c) = conn.as_ref() {
                    v["passive"] = passive::fetch(c, s.mac());
                }
                println!("{v}");
            } else {
                print!("{}", status::to_text(&s, fm));
            }
            EXIT_OK
        }
        Err(bus::BusError::Absent(m)) => {
            if json {
                println!("{}", status::absent_json(fm));
            } else {
                print!("{}", status::absent_text(fm));
            }
            eprintln!("akmctl: {m}");
            EXIT_ABSENT
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// `akmctl info`: register map + cached values; never reads the keyboard.
fn cmd_info(json: bool) -> u8 {
    let conn = bus::connect();
    let snap = conn.as_ref().map_err(|e| bus::BusError::Absent(e.to_string())).and_then(bus::get_state);
    let (snap, code) = match snap {
        Ok(s) => (Some(s), EXIT_OK),
        Err(bus::BusError::Absent(m)) => {
            eprintln!("akmctl: {m} (showing the static table, no cached values)");
            (None, EXIT_OK)
        }
        Err(e) => return fail(&e.to_string()),
    };
    let passive = match (conn.as_ref(), snap.as_ref()) {
        (Ok(c), Some(s)) => passive::fetch(c, s.mac()),
        _ => serde_json::Value::Null,
    };
    if json {
        println!("{}", info::to_json(snap.as_ref(), &passive));
    } else {
        print!("{}", info::to_text(snap.as_ref(), &passive));
    }
    code
}

/// `akmctl firmware`: version, latest known, status, source, table date.
fn cmd_firmware(json: bool) -> u8 {
    match daemon_snapshot() {
        Ok(Some(s)) => {
            if json {
                println!("{}", firmware::to_json(&s));
            } else {
                print!("{}", firmware::to_text(&s));
            }
            EXIT_OK
        }
        Ok(None) => {
            if json {
                println!("{}", serde_json::json!({"schema": 1, "daemon": false}));
            } else {
                print!("Daemon:       not running (the version is read by the daemon)\n{}", firmware::table_text());
            }
            EXIT_ABSENT
        }
        Err(e) => fail(&e),
    }
}

/// Snapshot of the daemon; `None` when it is not on the bus.
fn daemon_snapshot() -> Result<Option<akm_core::Snapshot>, String> {
    match bus::connect().map_err(|e| bus::BusError::Absent(e.to_string())).and_then(|c| bus::get_state(&c)) {
        Ok(s) => Ok(Some(s)),
        Err(bus::BusError::Absent(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn cmd_waybar() -> u8 {
    // Always valid JSON on stdout: a bar module must not show an error.
    match daemon_snapshot() {
        Ok(s) => println!("{}", render::waybar(s.as_ref())),
        Err(e) => {
            eprintln!("akmctl: {e}");
            println!("{}", render::waybar(None));
        }
    }
    EXIT_OK
}

fn cmd_metrics() -> u8 {
    match daemon_snapshot() {
        Ok(s) => {
            print!("{}", render::metrics(s.as_ref(), fnmode::read().ok()));
            if s.is_some() {
                EXIT_OK
            } else {
                EXIT_ABSENT
            }
        }
        Err(e) => fail(&e),
    }
}

/// Entries of the single history store (nothing if the file is missing).
fn load_history() -> Result<Vec<akm_core::history::HistoryEntry>, String> {
    let h = akm_core::history::History::open_default();
    if !h.path().exists() {
        return Err(format!(
            "no history at {} yet (the daemon writes it; `akmctl history import` brings in the old Python one)",
            h.path().display()
        ));
    }
    Ok(h.read())
}

fn bounds(f: &Filter, now: u64) -> Result<(Option<u64>, Option<u64>), String> {
    let p = |v: &Option<String>| v.as_deref().map(|s| when::parse_when(s, now)).transpose();
    Ok((p(&f.since)?, p(&f.until)?))
}

fn cmd_history(h: HistoryArgs) -> u8 {
    let now = akm_core::history::Clock::now(&akm_core::history::SystemClock);
    if let Some(HistoryCmd::Import { file }) = &h.cmd {
        let src = file.clone().unwrap_or_else(migrate::python_path);
        let dst = akm_core::history::default_path();
        return match migrate::import_file(&dst, &src) {
            Ok((added, ignored)) => {
                println!("{added} entries imported from {} into {} ({ignored} lines ignored)", src.display(), dst.display());
                EXIT_OK
            }
            Err(e) => fail(&format!("import from {}: {e}", src.display())),
        };
    }
    let (filter, csv) = match &h.cmd {
        Some(HistoryCmd::Export { filter, .. }) => (filter.clone(), true),
        _ => (h.filter.clone(), false),
    };
    let entries = match load_history() {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    let (since, until) = match bounds(&filter, now) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("akmctl: {e}");
            return EXIT_USAGE;
        }
    };
    let sel = histcmd::select(&entries, since, until, filter.last);
    let out = if csv {
        histcmd::to_csv(&sel)
    } else if h.json {
        format!("{}\n", histcmd::to_json(&sel))
    } else {
        histcmd::to_text(&sel)
    };
    if std::io::stdout().write_all(out.as_bytes()).is_err() {
        return EXIT_OK; // closed pipe
    }
    EXIT_OK
}

fn cmd_graph(span: Span) -> u8 {
    let now = akm_core::history::Clock::now(&akm_core::history::SystemClock);
    match load_history() {
        Ok(e) => {
            print!("{}", histcmd::graph(&e, now, span.seconds()));
            EXIT_OK
        }
        Err(e) => fail(&e),
    }
}

fn cmd_set_fnmode(mode: u8, persist: bool) -> u8 {
    let mut c = Proc::new(PKEXEC);
    c.arg(helper_path()).arg("set-fnmode").arg(mode.to_string());
    if persist {
        c.arg("--persist");
    }
    match c.status() {
        Ok(st) if st.success() => {
            match fnmode::read() {
                Ok(m) if m == mode => println!("Fn mode: {m} - {}", fnmode::label(m)),
                Ok(m) => return fail(&format!("write accepted but fnmode reads {m}, expected {mode}")),
                // --persist with hid_apple not loaded: the helper saved the
                // setting, nothing to read back yet (#153).
                Err(_) if persist && !std::path::Path::new(fnmode::SYSFS_FNMODE).exists() => {
                    println!("Fn mode {mode} saved ({}); hid_apple is not loaded, applied at next module load", fnmode::label(mode));
                    return EXIT_OK;
                }
                Err(e) => return fail(&e),
            }
            if !persist {
                eprintln!("note: not persistent across reboots (use --persist)");
            }
            EXIT_OK
        }
        Ok(st) => match st.code() {
            Some(126) => fail("authentication dismissed or not authorized"),
            Some(127) => fail("authentication failed or helper not found"),
            Some(c) => fail(&format!("helper failed (exit {c})")),
            None => fail("helper killed by a signal"),
        },
        Err(e) => fail(&format!("cannot run pkexec: {e}")),
    }
}

/// `rename <name>` / `rename --reset` (empty name): D-Bus `SetAlias`.
fn cmd_rename(name: &str, mac: Option<String>) -> u8 {
    let conn = match bus::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("akmctl: {e}");
            return EXIT_ABSENT;
        }
    };
    let mac = match mac {
        Some(m) => m,
        None => match bus::get_state(&conn) {
            Ok(s) => match s.mac() {
                Some(m) => m.to_string(),
                None => return fail("no keyboard known to the daemon (use --mac)"),
            },
            Err(bus::BusError::Absent(m)) => {
                eprintln!("akmctl: {m}");
                return EXIT_ABSENT;
            }
            Err(e) => return fail(&e.to_string()),
        },
    };
    match bus::set_alias(&conn, &mac, name) {
        Ok(now) if name.is_empty() => {
            println!("Name restored: {now}");
            EXIT_OK
        }
        Ok(now) => {
            println!("Name: {now}");
            EXIT_OK
        }
        Err(bus::BusError::Absent(m)) => {
            eprintln!("akmctl: {m}");
            EXIT_ABSENT
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn cmd_watch() -> u8 {
    let conn = match bus::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("akmctl: {e}");
            return EXIT_ABSENT;
        }
    };
    if !bus::daemon_present(&conn) {
        eprintln!("akmctl: daemon not running ({} absent on the session bus)", bus::BUS_NAME);
        return EXIT_ABSENT;
    }
    let proxy = match bus::proxy(&conn) {
        Ok(p) => p,
        Err(e) => return fail(&e.to_string()),
    };
    let signals = match proxy.receive_signal("StateChanged") {
        Ok(s) => s,
        Err(e) => return fail(&format!("subscribe: {e}")),
    };
    // Initial state first, so a consumer does not wait for the next change.
    if let Ok(s) = bus::get_state(&conn) {
        println!("{}", status::to_json(&s, fnmode::read().ok(), None));
        let _ = std::io::stdout().flush();
    }
    for msg in signals {
        let Ok((rev, json)) = msg.body().deserialize::<(u64, String)>() else {
            eprintln!("akmctl: malformed StateChanged signal ignored");
            continue;
        };
        match bus::parse_snapshot(&json) {
            Ok(s) => {
                let line = status::to_json(&s, fnmode::read().ok(), Some(rev));
                let mut out = std::io::stdout().lock();
                if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                    return EXIT_OK; // closed pipe (e.g. `| head -1`)
                }
            }
            Err(e) => eprintln!("akmctl: {e}"),
        }
    }
    // Stream ended: the bus connection dropped.
    fail("connection to the session bus lost")
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn pkexec_is_absolute_and_helper_is_the_packaged_path() {
        assert!(PKEXEC.starts_with('/'));
        assert!(HELPER.starts_with("/usr/lib/apple-kb-monitor/"));
    }

    /// The release source must not read AKM_HELPER outside `cfg(test)`.
    #[test]
    fn env_override_is_test_only() {
        let src = include_str!("main.rs");
        let (head, _) = src.split_once("#[cfg(not(test))]").unwrap();
        assert!(!head.contains("env::var(\"AKM_HELPER\")"));
        let after = src.split_once("#[cfg(not(test))]").unwrap().1;
        let release_fn = after.split_once("#[cfg(test)]").unwrap().0;
        assert!(!release_fn.contains("env::var"));
    }
}
