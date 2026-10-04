//! End-to-end runs of the built binary on fixture files.

use std::path::{Path, PathBuf};
use std::process::Command;

fn state_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("akmctl-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("apple-kb-monitor")).unwrap();
    d
}

/// The built akmctl, cut off from the user's buses (zbus falls back to `$XDG_RUNTIME_DIR/bus`)
/// and from the user's locale and catalogs: the assertions read English text.
fn akmctl(state: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_akmctl"));
    let no_bus = format!("unix:path={}", state.join("no-bus").display());
    c.env("XDG_STATE_HOME", state)
        .env("XDG_RUNTIME_DIR", state)
        .env("DBUS_SESSION_BUS_ADDRESS", &no_bus)
        .env("DBUS_SYSTEM_BUS_ADDRESS", &no_bus)
        .env_remove("DBUS_STARTER_ADDRESS")
        .env_remove("DBUS_STARTER_BUS_TYPE")
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env_remove("AKM_LOCALEDIR");
    c
}

fn run(state: &Path, args: &[&str]) -> (i32, String, String) {
    let o = akmctl(state).args(args).output().unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into(),
        String::from_utf8_lossy(&o.stderr).into(),
    )
}

const HIST: &str =
    "{\"ts\":1790000000,\"pct\":99.0,\"voltage\":3.002,\"schema\":2,\"mv_0x46\":3002}\n\
{\"ts\":1790003600,\"pct\":98.0,\"voltage\":2.99,\"schema\":2,\"mv_0x46\":2990}\n\
{\"ts\":1790007200,\"pct\":97.0}\n";

#[test]
fn history_csv_json_and_filters() {
    let d = state_dir("hist");
    std::fs::write(d.join("apple-kb-monitor/history.jsonl"), HIST).unwrap();
    let (c, out, _) = run(
        &d,
        &["history", "export", "--csv", "--since", "2026-09-21 15:00"],
    );
    assert_eq!(c, 0);
    assert_eq!(
        out,
        "time_utc,ts,battery_pct,voltage,mv_0x46,mv_0x49,event\n2026-09-21 15:13,1790003600,98,2.990,2990,,\n2026-09-21 16:13,1790007200,97,,,,\n"
    );
    let (c, out, _) = run(&d, &["history", "--json", "--last", "1"]);
    assert_eq!(c, 0);
    assert!(
        out.starts_with("[{")
            && out.trim_end().ends_with("}]")
            && out.matches("\"ts\"").count() == 1,
        "{out}"
    );
    let (c, out, _) = run(&d, &["history"]);
    assert!(c == 0 && out.lines().count() == 4, "{out}");
    let (c, _, err) = run(&d, &["history", "--since", "bogus"]);
    assert_eq!(c, 64);
    assert!(err.contains("invalid date \"bogus\": use 90m"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn missing_history_is_an_error_not_an_empty_success() {
    let d = state_dir("none");
    let (c, out, err) = run(&d, &["history", "export", "--csv"]);
    assert_eq!((c, out.as_str()), (1, ""));
    assert!(err.contains("no history"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn import_python_history_then_export() {
    let d = state_dir("imp");
    let py = d.join("apple-kb-monitor/history.jsonl");
    std::fs::write(
        &py,
        "{\"t\":\"2026-09-01T10:00:00\",\"bat\":80,\"fine\":78,\"volt\":2.9}\n",
    )
    .unwrap();
    let state = state_dir("imp-state");
    let o = akmctl(&state)
        .args(["history", "import"])
        .arg(&py)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let (c, out, _) = run(&state, &["history", "export", "--csv"]);
    assert_eq!(c, 0);
    assert!(
        out.lines().nth(1).unwrap().contains(",78,,,,"),
        "legacy voltage is not exported: {out}"
    );
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn led_refuses_numlock_on_and_bad_arguments() {
    let d = state_dir("led");
    let (c, _, err) = run(&d, &["led", "num", "on"]);
    assert_eq!(c, 1);
    assert!(err.contains("NumLock"), "{err}");
    assert_eq!(run(&d, &["led", "caps", "dim"]).0, 64);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn completions_and_man_list_the_new_commands() {
    let d = state_dir("compl");
    let (c, out, _) = run(&d, &["completions", "bash"]);
    assert_eq!(c, 0);
    for w in ["history", "graph", "waybar", "metrics", "led", "dump"] {
        assert!(out.contains(w), "{w}");
    }
    let (_, man, _) = run(&d, &["man"]);
    for w in [
        "akmctl\\-history",
        "akmctl\\-graph",
        "akmctl\\-waybar",
        "akmctl\\-metrics",
        "akmctl\\-led",
        "akmctl\\-dump",
    ] {
        assert!(man.contains(w), "{w}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn no_run_reaches_a_real_bus() {
    let src = include_str!("cli.rs");
    assert_eq!(
        src.matches(concat!("CARGO_BIN_", "EXE_akmctl")).count(),
        1,
        "every run goes through akmctl()"
    );
    let d = state_dir("nobus");
    let (c, out, err) = run(&d, &["rename", "--device-name", "--show"]);
    assert_eq!(c, 2, "a daemon answered: {out}{err}");
    assert!(err.contains("no session bus"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_device_name_with_show_or_restore_is_a_usage_error() {
    let d = state_dir("dn");
    let bk = d.join("bk.json");
    std::fs::write(&bk, "{}").unwrap();
    let bk = bk.to_str().unwrap();
    for args in [
        &["rename", "--device-name", "Foo", "--show"][..],
        &[
            "rename",
            "--device-name",
            "Foo",
            "--restore",
            bk,
            "--dry-run",
        ],
        &["rename", "--device-name", "Foo", "--restore", bk, "--yes"],
    ] {
        let (c, _, err) = run(&d, args);
        assert_eq!(c, 64, "{args:?}: {err}");
        assert!(err.contains("cannot be combined"), "{err}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// No command D-Bus-activates a stopped daemon: a private bus whose activation file only
/// leaves a mark, and no daemon on it.
#[test]
fn no_command_activates_the_daemon() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let d = state_dir("activation");
    let (svc, mark) = (d.join("services"), d.join("activated"));
    std::fs::create_dir_all(&svc).unwrap();
    std::fs::write(
        svc.join("com.agenceapi.AppleKbMonitor1.service"),
        format!(
            "[D-BUS Service]\nName=com.agenceapi.AppleKbMonitor1\nExec=/bin/sh -c 'echo x >> {}'\n",
            mark.display()
        ),
    )
    .unwrap();
    let conf = d.join("bus.conf");
    std::fs::write(
        &conf,
        format!(
            "<busconfig><type>session</type><listen>unix:dir={}</listen><auth>EXTERNAL</auth>\
             <servicedir>{}</servicedir><policy context=\"default\">\
             <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
             <allow own=\"*\"/></policy></busconfig>",
            d.display(),
            svc.display()
        ),
    )
    .unwrap();
    let Ok(mut bus) = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", conf.display()))
        .args(["--nofork", "--print-address"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        assert!(
            std::env::var_os("CI").is_none(),
            "dbus-daemon missing in CI"
        );
        return;
    };
    let mut addr = String::new();
    BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut addr)
        .unwrap();
    for cmd in ["status", "watch"] {
        let o = akmctl(&d)
            .env("DBUS_SESSION_BUS_ADDRESS", addr.trim())
            .arg(cmd)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "{cmd}");
    }
    let o = akmctl(&d)
        .env("DBUS_SESSION_BUS_ADDRESS", addr.trim())
        .arg("doctor")
        .output()
        .unwrap();
    let _ = bus.kill();
    let _ = bus.wait();
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(!mark.exists(), "the daemon was activated:\n{out}");
    assert!(out.contains("link keeper not reachable"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
