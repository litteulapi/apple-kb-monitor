//! End-to-end runs of the built binary on fixture files. No hardware, no
//! session bus needed (history is a plain file).

use std::path::PathBuf;
use std::process::Command;

fn state_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("akmctl-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("apple-kb-monitor")).unwrap();
    d
}

fn run(state: &PathBuf, args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_akmctl"))
        .args(args)
        .env("XDG_STATE_HOME", state)
        .env("XDG_RUNTIME_DIR", state)
        .output()
        .unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

const HIST: &str = "{\"ts\":1790000000,\"pct\":99.0,\"voltage\":3.002,\"schema\":2,\"mv_0x46\":3002}\n\
{\"ts\":1790003600,\"pct\":98.0,\"voltage\":2.99,\"schema\":2,\"mv_0x46\":2990}\n\
{\"ts\":1790007200,\"pct\":97.0}\n";

#[test]
fn history_csv_json_and_filters() {
    let d = state_dir("hist");
    std::fs::write(d.join("apple-kb-monitor/history.jsonl"), HIST).unwrap();
    let (c, out, _) = run(&d, &["history", "export", "--csv", "--since", "2026-09-21 15:00"]);
    assert_eq!(c, 0);
    assert_eq!(
        out,
        "time_utc,ts,battery_pct,voltage,mv_0x46,mv_0x49,event\n2026-09-21 15:13,1790003600,98,2.990,2990,,\n2026-09-21 16:13,1790007200,97,,,,\n"
    );
    let (c, out, _) = run(&d, &["history", "--json", "--last", "1"]);
    assert_eq!(c, 0);
    assert!(out.starts_with("[{") && out.trim_end().ends_with("}]") && out.matches("\"ts\"").count() == 1, "{out}");
    let (c, out, _) = run(&d, &["history"]);
    assert!(c == 0 && out.lines().count() == 4, "{out}");
    assert_eq!(run(&d, &["history", "--since", "bogus"]).0, 64);
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
    std::fs::write(&py, "{\"t\":\"2026-09-01T10:00:00\",\"bat\":80,\"fine\":78,\"volt\":2.9}\n").unwrap();
    // Python file is at $XDG_RUNTIME_DIR/apple-kb-monitor/history.jsonl; the
    // Rust store is $XDG_STATE_HOME/apple-kb-monitor/history.jsonl: here both
    // variables point to the same directory, so use two directories.
    let state = state_dir("imp-state");
    let o = Command::new(env!("CARGO_BIN_EXE_akmctl"))
        .args(["history", "import"])
        .env("XDG_STATE_HOME", &state)
        .env("XDG_RUNTIME_DIR", &d)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let (c, out, _) = run(&state, &["history", "export", "--csv"]);
    assert_eq!(c, 0);
    assert!(out.lines().nth(1).unwrap().contains(",78,,,,"), "legacy voltage is not exported: {out}");
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
    for w in ["akmctl\\-history", "akmctl\\-graph", "akmctl\\-waybar", "akmctl\\-metrics", "akmctl\\-led", "akmctl\\-dump"] {
        assert!(man.contains(w), "{w}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
