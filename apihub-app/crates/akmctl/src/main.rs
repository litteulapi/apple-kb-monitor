//! `akmctl` — command-line client of apple-kb-monitord.

mod bus;
mod cli;
mod fnmode;
mod status;

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
    let snap = bus::connect().and_then(|c| bus::get_state(&c));
    match snap {
        Ok(s) => {
            if json {
                println!("{}", status::to_json(&s, fm, None));
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
