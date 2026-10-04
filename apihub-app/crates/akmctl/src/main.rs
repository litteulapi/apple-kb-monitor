//! `akmctl` — command-line client of apple-kb-monitord.

mod bus;
mod cli;
mod cli_i18n;
#[cfg(test)]
mod cli_strings;
mod devnamecmd;
mod doctor;
#[allow(dead_code)] // source shared with akm-helper; akmctl uses only part of it
#[path = "../../akm-helper/src/doctor_fix.rs"]
mod doctor_fix;
mod doctorfix;
mod dump;
mod firmware;
mod fnmode;
mod forget;
mod hid_control;
mod histcmd;
mod info;
mod kde;
mod keymapcmd;
mod keys;
mod keyslive;
mod ledcmd;
mod migrate;
mod passive;
mod pkexec;
mod render;
mod repair;
mod selftest;
mod shutdown_notify;
mod status;
mod when;

use akm_core::{tr, N_};
use std::io::Write;
use std::process::{Command as Proc, ExitCode};

use clap::{CommandFactory, FromArgMatches};
use cli::{
    Cli, Command, Filter, GetCmd, HistoryArgs, HistoryCmd, SetCmd, Span, EXIT_ABSENT, EXIT_ERROR,
    EXIT_OK, EXIT_USAGE,
};

use akm_core::paths::{HELPER, PKEXEC};

/// `--json` anywhere and the `watch` subcommand (JSON lines) stay untranslated; argv may hold
/// non-UTF-8 bytes. akmctl has no global option, so the subcommand is the first non-option.
fn untranslated(args: impl Iterator<Item = std::ffi::OsString>) -> bool {
    let mut subcommand = None;
    for a in args {
        if a == "--json" {
            return true;
        }
        if subcommand.is_none() && !a.as_encoded_bytes().starts_with(b"-") {
            subcommand = Some(a);
        }
    }
    subcommand.is_some_and(|a| a == "watch")
}

fn main() -> ExitCode {
    if !untranslated(std::env::args_os().skip(1)) {
        akm_core::i18n::init();
    }
    akm_core::read_policy::share_hw_access(true);
    let parsed = cli_i18n::localized(Cli::command())
        .try_get_matches()
        .and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(c) => c,
        Err(e) => {
            let ok = !e.use_stderr();
            let _ = e.print();
            return ExitCode::from(if ok { EXIT_OK } else { EXIT_USAGE });
        }
    };
    ExitCode::from(run(cli.command))
}

#[allow(clippy::too_many_lines)] // one arm per subcommand
fn run(cmd: Command) -> u8 {
    match cmd {
        Command::Status { json } => cmd_status(json),
        Command::Get {
            what: GetCmd::Fnmode,
        } => match fnmode::read() {
            Ok(m) => {
                println!("{m}");
                EXIT_OK
            }
            Err(e) => fail(&e),
        },
        Command::Get {
            what: GetCmd::Param { name },
        } => keymapcmd::cmd_get_param(name.as_deref()),
        Command::Set {
            what: SetCmd::Fnmode { mode, persist },
        } => cmd_set_fnmode(mode, persist),
        Command::Set {
            what:
                SetCmd::Param {
                    name,
                    value,
                    persist,
                },
        } => {
            let p = akm_core::keymap::kernel_param(&name).expect("validated by clap");
            match akm_core::keymap::parse_param_value(p, &value) {
                Ok(v) => keymapcmd::cmd_set_param(&name, v, persist),
                Err(e) => {
                    eprintln!("akmctl: {e}");
                    EXIT_USAGE
                }
            }
        }
        Command::Keys { live: true, .. } => keyslive::command(),
        Command::Keys {
            check, all, json, ..
        } => keymapcmd::cmd_keys(check, all, json),
        Command::Keymap { cmd } => keymapcmd::run(cmd),
        Command::Rename {
            device_name: Some(dn),
            show,
            restore,
            write_device_name: _,
            dry_run,
            check,
            yes,
            verbose,
            mac,
            ..
        } => {
            let mode = if dry_run {
                devnamecmd::Mode::DryRun
            } else if check {
                devnamecmd::Mode::Check
            } else {
                devnamecmd::Mode::Write
            };
            let action = match (dn, show, restore) {
                (Some(_), true, _) | (Some(_), _, Some(_)) => {
                    eprintln!(
                        "akmctl: {}",
                        tr!("--device-name NAME cannot be combined with --show or --restore")
                    );
                    return EXIT_USAGE;
                }
                (None, true, _) => devnamecmd::Action::Show,
                (None, false, Some(file)) => devnamecmd::Action::Restore { file, mode },
                (Some(name), _, None) => devnamecmd::Action::Rename { name, mode },
                (None, false, None) => {
                    eprintln!(
                        "akmctl: {}",
                        tr!("--device-name needs a name, --show or --restore")
                    );
                    return EXIT_USAGE;
                }
            };
            devnamecmd::run(action, mac, devnamecmd::Opts { yes, verbose })
        }
        Command::Rename { name, mac, .. } => cmd_rename(name.as_deref().unwrap_or(""), mac),
        Command::Watch => cmd_watch(),
        Command::History(h) => cmd_history(&h),
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
        Command::Doctor {
            mac,
            fix: true,
            dry_run,
            restart_services,
            optional,
            ..
        } => doctorfix::command(
            mac.as_deref(),
            doctorfix::Options {
                dry_run,
                restart: restart_services,
                optional,
            },
        ),
        Command::Doctor { json, mac, .. } => {
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
        Command::Selftest(a) => selftest::run_selftest(&a),
        Command::ShutdownNotify { only_if_stopping } => shutdown_notify::run(only_if_stopping),
        Command::HidControl { op, mac, dry_run } => hid_control::run(op, mac.as_deref(), dry_run),
        Command::Completions { shell } => {
            let mut c = Cli::command();
            clap_complete::generate(
                clap_complete::Shell::from(shell),
                &mut c,
                "akmctl",
                &mut std::io::stdout(),
            );
            EXIT_OK
        }
        Command::Man => {
            let mut buf = Vec::new();
            if clap_mangen::Man::new(Cli::command())
                .render(&mut buf)
                .is_err()
            {
                return fail(&tr!("cannot render the manual page"));
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
    let snap = conn
        .as_ref()
        .map_err(|e| bus::BusError::Absent(e.to_string()))
        .and_then(bus::get_state);
    match snap {
        Ok(s) => {
            if json {
                let mut v = status::to_json(&s, fm, None);
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
    let snap = conn
        .as_ref()
        .map_err(|e| bus::BusError::Absent(e.to_string()))
        .and_then(bus::get_state);
    let (snap, code) = match snap {
        Ok(s) => (Some(s), EXIT_OK),
        Err(bus::BusError::Absent(m)) => {
            eprintln!(
                "akmctl: {}",
                tr!("{m} (showing the static table, no cached values)", m = m)
            );
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
                print!(
                    "{:<13} {}\n{}",
                    tr!("Daemon:"),
                    tr!("not running (the version is read by the daemon)"),
                    firmware::table_text()
                );
            }
            EXIT_ABSENT
        }
        Err(e) => fail(&e),
    }
}

fn daemon_snapshot() -> Result<Option<akm_core::Snapshot>, String> {
    match bus::connect()
        .map_err(|e| bus::BusError::Absent(e.to_string()))
        .and_then(|c| bus::get_state(&c))
    {
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

fn load_history() -> Result<Vec<akm_core::history::HistoryEntry>, String> {
    let h = akm_core::history::History::open_default();
    if !h.path().exists() {
        return Err(tr!(
            "no history at {path} yet (the daemon writes it)",
            path = h.path().display()
        ));
    }
    Ok(h.read())
}

fn bounds(f: &Filter, now: u64) -> Result<(Option<u64>, Option<u64>), String> {
    let p = |v: &Option<String>| v.as_deref().map(|s| when::parse_when(s, now)).transpose();
    Ok((p(&f.since)?, p(&f.until)?))
}

fn cmd_history(h: &HistoryArgs) -> u8 {
    let now = akm_core::history::Clock::now(&akm_core::history::SystemClock);
    if let Some(HistoryCmd::Import { file }) = &h.cmd {
        let src = file.clone();
        let dst = akm_core::history::default_path();
        return match migrate::import_file(&dst, &src) {
            Ok((added, ignored)) => {
                println!(
                    "{}",
                    tr!(
                        "{added} entries imported from {src} into {dst} ({ignored} lines ignored)",
                        added = added,
                        src = src.display(),
                        dst = dst.display(),
                        ignored = ignored
                    )
                );
                EXIT_OK
            }
            Err(e) => fail(&tr!("import from {src}: {e}", src = src.display(), e = e)),
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
    let _ = std::io::stdout().write_all(out.as_bytes()); // a closed pipe is not an error
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
    c.arg(HELPER).arg("set-fnmode").arg(mode.to_string());
    if persist {
        c.arg("--persist");
    }
    match c.status() {
        Ok(st) if st.success() => {
            match fnmode::read() {
                Ok(m) if m == mode => println!(
                    "{}",
                    tr!("Fn mode: {m} - {label}", m = m, label = fnmode::label(m))
                ),
                Ok(m) => {
                    return fail(&tr!(
                        "write accepted but fnmode reads {m}, expected {mode}",
                        m = m,
                        mode = mode
                    ))
                }
                Err(_) if persist && !std::path::Path::new(fnmode::SYSFS_FNMODE).exists() => {
                    println!(
                        "{}",
                        tr!(
                            "Fn mode {mode} saved ({label}); hid_apple is not loaded, applied at next module load",
                            mode = mode,
                            label = fnmode::label(mode)
                        )
                    );
                    return EXIT_OK;
                }
                Err(e) => return fail(&e),
            }
            if !persist {
                eprintln!(
                    "{}",
                    tr!("note: not persistent across reboots (use --persist)")
                );
            }
            EXIT_OK
        }
        Ok(st) => match (st.code(), pkexec::failure(st.code(), HELPER)) {
            (_, Some(e)) => fail(&e),
            (Some(c), None) => fail(&tr!("helper failed (exit {c})", c = c)),
            (None, None) => fail(&tr!("helper killed by a signal")),
        },
        Err(e) => fail(&tr!("cannot run pkexec: {e}", e = e)),
    }
}

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
                None => return fail(&tr!("no keyboard known to the daemon (use --mac)")),
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
            println!("{}", tr!("Name restored: {now}", now = now));
            EXIT_OK
        }
        Ok(now) => {
            println!("{}", tr!("Name: {now}", now = now));
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
    let proxy = match bus::daemon(&conn) {
        Ok(p) => p,
        Err(e @ bus::BusError::Absent(_)) => {
            eprintln!("akmctl: {e}");
            return EXIT_ABSENT;
        }
        Err(e) => return fail(&e.to_string()),
    };
    let signals = match proxy.receive_signal("StateChanged") {
        Ok(s) => s,
        Err(e) => return fail(&tr!("cannot subscribe to the daemon's signals: {e}", e = e)),
    };
    if let Ok(s) = bus::get_state(&conn) {
        println!("{}", status::to_json(&s, fnmode::read().ok(), None));
        let _ = std::io::stdout().flush();
    }
    for msg in signals {
        let Ok((rev, json)) = msg.body().deserialize::<(u64, String)>() else {
            eprintln!("akmctl: {}", tr!("malformed StateChanged signal ignored"));
            continue;
        };
        match bus::parse_snapshot(&json) {
            Ok(s) => {
                let line = status::to_json(&s, fnmode::read().ok(), Some(rev));
                let mut out = std::io::stdout().lock();
                if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                    return EXIT_OK; // closed pipe (e.g. `| head -1`)
                }
            }
            Err(e) => eprintln!("akmctl: {e}"),
        }
    }
    fail(&tr!("connection to the session bus lost"))
}

#[cfg(test)]
mod argv_tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn json_and_watch_skip_translation_and_bad_utf8_does_not_panic() {
        let v = |a: &[&str]| a.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(super::untranslated(v(&["status", "--json"]).into_iter()));
        assert!(super::untranslated(v(&["watch"]).into_iter()));
        assert!(!super::untranslated(v(&["status"]).into_iter()));
        assert!(!super::untranslated(v(&["rename", "watch"]).into_iter()));
        assert!(!super::untranslated(
            v(&["history", "--since", "watch"]).into_iter()
        ));
        assert!(super::untranslated(
            v(&["rename", "watch", "--json"]).into_iter()
        ));
        let bad = vec![OsString::from("status"), OsString::from_vec(vec![0xff])];
        assert!(!super::untranslated(bad.into_iter()));
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn pkexec_is_absolute_and_helper_is_the_packaged_path() {
        assert!(PKEXEC.starts_with('/'));
        assert!(HELPER.starts_with("/usr/lib/apple-kb-monitor/"));
    }
}
