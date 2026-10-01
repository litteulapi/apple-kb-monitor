//! `akm-helper` — the only privileged piece of apple-kb-monitor.
//!
//! Installed at `/usr/lib/apple-kb-monitor/akm-helper` and started through
//! `pkexec` (polkit action `com.agenceapi.AppleKbMonitor.set-fnmode`).
//!
//! ```text
//! akm-helper set-fnmode <0|1|2|3> [--persist]
//! ```
//!
//! * writes `/sys/module/hid_apple/parameters/fnmode` (root-only, mode 644);
//! * `--persist` also rewrites the `fnmode=` token of `/etc/modprobe.d/hid_apple.conf`
//!   (other options and comments of the file are kept; atomic rename).
//!
//! Security stance: the argument list is a strict whitelist (one verb, one
//! value in 0..=3, one optional flag); no path, no environment variable and
//! no stdin is ever used to pick a file or a value.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

const SYSFS_FNMODE: &str = "/sys/module/hid_apple/parameters/fnmode";
const MODPROBE_CONF: &str = "/etc/modprobe.d/hid_apple.conf";

/// Exit codes: 0 OK, 1 write error, 64 invalid usage (sysexits EX_USAGE).
const EX_USAGE: u8 = 64;

#[derive(Debug, PartialEq, Eq)]
struct Request {
    mode: u8,
    persist: bool,
}

/// Strict parser: exactly `set-fnmode <0..=3> [--persist]`.
fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Request, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let (verb, val, flag) = match a.as_slice() {
        [v, n] => (*v, *n, None),
        [v, n, f] => (*v, *n, Some(*f)),
        _ => return Err("usage: akm-helper set-fnmode <0-3> [--persist]".into()),
    };
    if verb != "set-fnmode" {
        return Err(format!("unknown command {verb:?}"));
    }
    let persist = match flag {
        None => false,
        Some("--persist") => true,
        Some(f) => return Err(format!("unknown option {f:?}")),
    };
    // Exactly one ASCII digit: rejects "", "+1", "01", " 1", "1\n", "٢", "10"...
    let mode = match val.as_bytes() {
        [d @ b'0'..=b'3'] => d - b'0',
        _ => return Err(format!("invalid fnmode {val:?} (expected 0, 1, 2 or 3)")),
    };
    Ok(Request { mode, persist })
}

/// New content of the modprobe file: replaces the `fnmode=` token of every
/// `options hid_apple ...` line, or appends one line. Other lines untouched.
fn merge_fnmode(existing: &str, mode: u8) -> String {
    let mut out = String::new();
    let mut done = false;
    for line in existing.lines() {
        let mut toks = line.split_whitespace();
        if toks.next() == Some("options") && toks.next() == Some("hid_apple") {
            let mut replaced = false;
            let mut parts: Vec<String> = Vec::new();
            for t in line.split_whitespace() {
                if t.starts_with("fnmode=") {
                    if !replaced {
                        parts.push(format!("fnmode={mode}"));
                        replaced = true;
                    }
                } else {
                    parts.push(t.to_string());
                }
            }
            if !replaced {
                parts.push(format!("fnmode={mode}"));
            }
            out.push_str(&parts.join(" "));
            done = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !done {
        out.push_str(&format!("options hid_apple fnmode={mode}\n"));
    }
    out
}

fn persist(path: &Path, mode: u8) -> std::io::Result<()> {
    let existing = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let new = merge_fnmode(&existing, mode);
    let tmp = path.with_extension("conf.akm-tmp");
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .or_else(|_| {
                // stale temp from a crashed run
                let _ = fs::remove_file(&tmp);
                OpenOptions::new().write(true).create_new(true).open(&tmp)
            })?;
        f.write_all(new.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn set_sysfs(path: &Path, mode: u8) -> std::io::Result<()> {
    // No create: the parameter must exist (hid_apple loaded).
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.write_all(format!("{mode}\n").as_bytes())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse_args(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akm-helper: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    if let Err(e) = set_sysfs(Path::new(SYSFS_FNMODE), req.mode) {
        eprintln!("akm-helper: cannot write {SYSFS_FNMODE}: {e}");
        return ExitCode::from(1);
    }
    if req.persist {
        if let Err(e) = persist(Path::new(MODPROBE_CONF), req.mode) {
            eprintln!("akm-helper: fnmode applied but {MODPROBE_CONF} not updated: {e}");
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        for n in 0..=3u8 {
            assert_eq!(
                parse_args(&["set-fnmode", &n.to_string()]),
                Ok(Request { mode: n, persist: false })
            );
        }
        assert_eq!(
            parse_args(&["set-fnmode", "2", "--persist"]),
            Ok(Request { mode: 2, persist: true })
        );
    }

    #[test]
    fn rejects_invalid_values() {
        for v in ["4", "9", "10", "-1", "+1", "01", "", " 1", "1\n", "a", "1;", "١", "0x1", "../x"] {
            assert!(parse_args(&["set-fnmode", v]).is_err(), "{v:?}");
        }
    }

    #[test]
    fn rejects_bad_shape() {
        let none: [&str; 0] = [];
        assert!(parse_args(&none).is_err());
        assert!(parse_args(&["set-fnmode"]).is_err());
        assert!(parse_args(&["set-isolayout", "1"]).is_err());
        assert!(parse_args(&["set-fnmode", "1", "--force"]).is_err());
        assert!(parse_args(&["set-fnmode", "1", "--persist", "x"]).is_err());
        assert!(parse_args(&["/etc/passwd", "1"]).is_err());
    }

    #[test]
    fn merge_creates_and_replaces() {
        assert_eq!(merge_fnmode("", 2), "options hid_apple fnmode=2\n");
        assert_eq!(merge_fnmode("options hid_apple fnmode=1\n", 3), "options hid_apple fnmode=3\n");
        assert_eq!(
            merge_fnmode("# c\noptions hid_apple iso_layout=0 fnmode=1 swap_opt_cmd=1\noptions foo a=1\n", 2),
            "# c\noptions hid_apple iso_layout=0 fnmode=2 swap_opt_cmd=1\noptions foo a=1\n"
        );
        assert_eq!(
            merge_fnmode("options hid_apple iso_layout=1\n", 0),
            "options hid_apple iso_layout=1 fnmode=0\n"
        );
        assert_eq!(merge_fnmode("# only comment\n", 1), "# only comment\noptions hid_apple fnmode=1\n");
    }

    #[test]
    fn persist_roundtrip_in_tempdir() {
        let dir = std::env::temp_dir().join(format!("akm-helper-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("hid_apple.conf");
        persist(&p, 2).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "options hid_apple fnmode=2\n");
        persist(&p, 0).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "options hid_apple fnmode=0\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
