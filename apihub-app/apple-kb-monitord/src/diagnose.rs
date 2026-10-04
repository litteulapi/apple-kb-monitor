//! D-Bus `Diagnose()`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use akm_core::hid_params::Param;

/// Version of the JSON layout.
pub const SCHEMA: u32 = 1;
/// Longest run of one external program.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

/// One row of the diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: &'static str,
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

/// How a bounded program run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// Exit status 0; first line of its standard output.
    Ok(String),
    /// Non-zero exit; exit code and standard error.
    Failed(i32, String),
    Timeout,
    NotFound,
}

/// What the checks look at: the real system, or a fixture in tests.
pub trait Probe: Send + Sync {
    fn run(&self, bin: &str, args: &[&str]) -> Run;
    fn path(&self, absolute: &str) -> PathBuf;
    fn apple_hidraw(&self) -> Option<String>;
    /// # Errors
    /// Why `path` cannot be opened for reading.
    fn readable(&self, path: &str) -> Result<(), String>;
    fn executable(&self, path: &str) -> bool;
    /// File capability `cap_net_admin` present (rssi-helper).
    fn cap_net_admin(&self, path: &str) -> bool {
        akm_core::rssi::file_has_cap_net_admin(path)
    }
    fn rssi_issue(&self) -> Option<akm_core::rssi::RssiIssue>;
}

/// The real system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProbe;

/// Run a program with a deadline ([`crate::privileged::run_bounded`]).
fn run_probe(cmd: &mut Command, timeout: Duration) -> Run {
    match crate::privileged::run_bounded(cmd, timeout) {
        Err(_) => Run::NotFound,
        Ok(f) => match f.status {
            None => Run::Timeout,
            Some(st) if st.success() => {
                Run::Ok(f.out.lines().next().unwrap_or("").trim().to_string())
            }
            Some(st) => Run::Failed(st.code().unwrap_or(-1), f.err.trim().to_string()),
        },
    }
}

impl Probe for SystemProbe {
    fn run(&self, bin: &str, args: &[&str]) -> Run {
        run_probe(Command::new(bin).args(args), COMMAND_TIMEOUT)
    }
    fn path(&self, absolute: &str) -> PathBuf {
        PathBuf::from(absolute)
    }
    fn apple_hidraw(&self) -> Option<String> {
        akm_core::hidraw::find_apple_hidraw()
    }
    fn executable(&self, path: &str) -> bool {
        let Ok(c) = std::ffi::CString::new(path) else {
            return false;
        };
        // SAFETY: `c` is a valid NUL-terminated string for the whole call.
        unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 }
    }
    fn rssi_issue(&self) -> Option<akm_core::rssi::RssiIssue> {
        akm_core::rssi::last_issue()
    }
    fn readable(&self, path: &str) -> Result<(), String> {
        let c = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
        // SAFETY: `c` is a valid NUL-terminated string for the whole call.
        if unsafe { libc::access(c.as_ptr(), libc::R_OK) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}

/// `fnmode` set by the `options hid_apple... fnmode=N` lines of one modprobe.d file.
#[must_use]
pub fn parse_modprobe(content: &str) -> Option<i32> {
    let mut found = None;
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        if words.next() != Some("options") || words.next() != Some("hid_apple") {
            continue;
        }
        for w in words {
            if let Some(n) = w
                .strip_prefix("fnmode=")
                .and_then(|v| v.parse::<i32>().ok())
                .filter(|n| Param::FnMode.valid(*n))
            {
                found = Some(n);
            }
        }
    }
    found
}

/// Configured `fnmode` over every `*.conf` of `dir`, in modprobe order.
#[must_use]
pub fn configured_fnmode(dir: &Path) -> Option<i32> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "conf"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|c| parse_modprobe(&c))
        .next_back()
}

fn program(p: &dyn Probe, id: &'static str, label: &str, bin: &str) -> Check {
    let (ok, detail) = match p.run(bin, &["--version"]) {
        Run::Ok(first) if first.is_empty() => (true, format!("{bin}: {}", tr!("OK"))),
        Run::Ok(first) => {
            let v = first
                .strip_prefix(&format!("{bin}:"))
                .map_or(first.as_str(), str::trim_start);
            (true, format!("{bin}: {v}"))
        }
        Run::Failed(_, err) if err.to_ascii_lowercase().contains("usage") => {
            (true, format!("{bin}: {}", tr!("installed")))
        }
        Run::Failed(code, _) => (
            false,
            tr!("{bin}: exit code {code}", bin = bin, code = code),
        ),
        Run::Timeout => (
            false,
            tr!(
                "{bin}: no answer within {secs} s (killed)",
                secs = COMMAND_TIMEOUT.as_secs(),
                bin = bin
            ),
        ),
        Run::NotFound => (false, format!("{bin}: {}", tr!("NOT FOUND"))),
    };
    Check {
        id,
        label: label.to_string(),
        ok,
        detail,
    }
}

fn file_check(p: &dyn Probe, id: &'static str, label: &str, path: &str, present: &str) -> Check {
    let ok = p.path(path).exists();
    Check {
        id,
        label: label.to_string(),
        ok,
        detail: if ok {
            present.to_string()
        } else {
            format!("{path}: {}", tr!("NOT FOUND"))
        },
    }
}

/// Every check, in the order of the Diag tab.
#[allow(clippy::too_many_lines)] // flat list of independent checks
pub fn run_checks(p: &dyn Probe, on_bus: bool) -> Vec<Check> {
    let mut out = Vec::new();
    // The daemon itself: it is the one answering.
    out.push(Check {
        id: "daemon",
        label: tr!("Monitor daemon"),
        ok: true,
        detail: format!("apple-kb-monitord {}", akm_core::PKG_VERSION),
    });
    out.push(program(
        p,
        "bluetoothctl",
        &tr!("BlueZ CLI"),
        "bluetoothctl",
    ));
    let active = matches!(
        p.run(
            "systemctl",
            &["--user", "is-active", "apple-kb-monitord.service"]
        ),
        Run::Ok(_)
    );
    out.push(Check {
        id: "service",
        label: "apple-kb-monitord.service".into(),
        ok: active,
        detail: if active {
            tr!("active (running)")
        } else {
            tr!("inactive / not found (daemon started by hand?)")
        },
    });
    out.push(Check {
        id: "dbus",
        label: format!("D-Bus {}", crate::service::BUS_NAME),
        ok: on_bus,
        detail: if on_bus {
            tr!("daemon reachable on the session bus")
        } else {
            tr!("daemon without its session interface")
        },
    });
    // hidraw node of the keyboard: present AND readable (udev `uaccess`).
    let (hid_ok, hid_detail) = match p.apple_hidraw() {
        None => (
            false,
            tr!("no Apple hidraw device found (keyboard off or not paired?)"),
        ),
        Some(path) => match p.readable(&path) {
            Ok(()) => (true, format!("{path}: {}", tr!("readable"))),
            Err(e) => (
                false,
                format!(
                    "{path}: {e} \u{2014} {}",
                    tr!("check the udev uaccess rule")
                ),
            ),
        },
    };
    out.push(Check {
        id: "hidraw",
        label: tr!("hidraw readable"),
        ok: hid_ok,
        detail: hid_detail,
    });
    // Key mapping: udev hwdb written by `akmctl keymap apply`.
    let hwdb = akm_core::keymap::HWDB_PATH;
    let (km_ok, mut km_detail) = match akm_core::keymap::read_installed(&p.path(hwdb)) {
        Ok(r) if r.is_empty() => (
            true,
            tr!("kernel mapping, no hwdb installed (optional: akmctl keymap)"),
        ),
        Ok(r) => (
            true,
            trn!(
                "{hwdb}: {count} model remapped",
                "{hwdb}: {count} models remapped",
                r.len(),
                count = r.len(),
                hwdb = hwdb
            ),
        ),
        Err(e) => (
            false,
            format!(
                "{e} \u{2014} {}",
                tr!("reinstall: akmctl keymap apply, or remove: akmctl keymap reset")
            ),
        ),
    };
    let keyd_active = matches!(
        p.run("systemctl", &["is-active", "--quiet", "keyd.service"]),
        Run::Ok(_)
    );
    if keyd_active && p.path("/etc/keyd/apple-keyboard.conf").exists() {
        km_detail.push_str(&tr!(
            " \u{b7} keyd also active with /etc/keyd/apple-keyboard.conf (optional)"
        ));
    }
    out.push(Check {
        id: "keymap",
        label: tr!("Key mapping"),
        ok: km_ok,
        detail: km_detail,
    });
    out.push(file_check(
        p,
        "udev",
        &tr!("udev rules"),
        "/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules",
        &tr!("70-apple-kb-hidraw.rules installed"),
    ));
    // hid_apple fnmode: the value applied (sysfs) against the configured one.
    let live = Param::FnMode.read_in(&p.path(akm_core::hid_params::SYSFS_DIR));
    let configured = configured_fnmode(&p.path("/etc/modprobe.d"));
    let show = |v: Option<i32>, none: &str| v.map_or(none.to_string(), |n| n.to_string());
    let state = tr!(
        "applied (sysfs) = {} \u{b7} configured (modprobe.d) = {}",
        show(live, &tr!("unreadable (hid_apple not loaded?)")),
        show(configured, &tr!("none"))
    );
    let (fn_ok, fn_detail) = match (live, configured) {
        (Some(l), Some(c)) if l == c => (true, state),
        (Some(_), Some(_)) => (
            false,
            format!(
                "{state} \u{2014} {}",
                tr!("differ: reload hid_apple or reboot")
            ),
        ),
        (Some(_), None) => (
            true,
            format!(
                "{state} ({})",
                tr!("not persistent: akmctl set fnmode N --persist")
            ),
        ),
        (None, _) => (false, state),
    };
    out.push(Check {
        id: "fnmode",
        label: "hid_apple fnmode".into(),
        ok: fn_ok,
        detail: fn_detail,
    });
    out.push(rssi_check(p));
    out
}

fn rssi_check(p: &dyn Probe) -> Check {
    use akm_core::rssi;
    let path = p.path(rssi::HELPER_PATH);
    let code = if !path.exists() {
        Some(rssi::CODE_HELPER_MISSING.to_string())
    } else if let Some(i) = p.rssi_issue() {
        Some(i.code)
    } else if !p.executable(&path.to_string_lossy()) || !p.cap_net_admin(&path.to_string_lossy()) {
        Some(rssi::CODE_HELPER_FAILED.to_string())
    } else {
        None
    };
    Check {
        id: "rssi_helper",
        label: tr!("Signal measure"),
        ok: code.is_none(),
        detail: match code {
            None => tr!("rssi-helper executable, capability cap_net_admin present"),
            Some(c) => {
                let (why, fix) = rssi::explain(&c);
                format!("{why}. {} {fix}", tr!("Fix:"))
            }
        },
    }
}

/// The JSON document of `Diagnose()`.
#[must_use]
pub fn to_json(checks: &[Check]) -> serde_json::Value {
    serde_json::json!({
        "schema": SCHEMA,
        "daemon_version": akm_core::PKG_VERSION,
        "passed": checks.iter().filter(|c| c.ok).count(),
        "total": checks.len(),
        "checks": checks
            .iter()
            .map(|c| serde_json::json!({
                "id": c.id, "label": c.label, "ok": c.ok, "detail": c.detail,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    pub struct Fake {
        pub root: PathBuf,
        pub hidraw: Option<String>,
        pub readable: Result<(), String>,
        pub runs: Mutex<Vec<String>>,
        pub service_active: bool,
        pub bluetoothctl: Run,
        pub helper_exec: bool,
        pub helper_cap: bool,
        pub rssi_issue: Option<akm_core::rssi::RssiIssue>,
    }

    impl Fake {
        pub fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("akm-diag-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self {
                root,
                hidraw: None,
                readable: Ok(()),
                runs: Mutex::new(Vec::new()),
                service_active: false,
                bluetoothctl: Run::NotFound,
                helper_exec: true,
                helper_cap: true,
                rssi_issue: None,
            }
        }
        fn write(&self, path: &str, content: &str) {
            let p = self.path(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Probe for Fake {
        fn run(&self, bin: &str, args: &[&str]) -> Run {
            self.runs
                .lock()
                .unwrap()
                .push(format!("{bin} {}", args.join(" ")));
            match (bin, args.first().copied()) {
                ("bluetoothctl", _) => self.bluetoothctl.clone(),
                ("systemctl", Some("--user")) if self.service_active => Run::Ok("active".into()),
                _ => Run::Failed(3, String::new()),
            }
        }
        fn path(&self, absolute: &str) -> PathBuf {
            self.root.join(absolute.trim_start_matches('/'))
        }
        fn apple_hidraw(&self) -> Option<String> {
            self.hidraw.clone()
        }
        fn readable(&self, _path: &str) -> Result<(), String> {
            self.readable.clone()
        }
        fn cap_net_admin(&self, _path: &str) -> bool {
            self.helper_cap
        }
        fn executable(&self, _path: &str) -> bool {
            self.helper_exec
        }
        fn rssi_issue(&self) -> Option<akm_core::rssi::RssiIssue> {
            self.rssi_issue.clone()
        }
    }

    #[test]
    fn the_signal_check_follows_the_real_read() {
        use akm_core::rssi;
        let mut f = Fake::new("rssi");
        f.write("/usr/lib/apple-kb-monitor/rssi-helper", "");
        let ok = |f: &Fake| by_id(&run_checks(f, true), "rssi_helper").clone();
        assert!(ok(&f).ok, "{:?}", ok(&f));
        f.rssi_issue = Some(rssi::classify(&rssi::RssiError::Denied));
        let c = ok(&f);
        assert!(!c.ok);
        assert!(
            !c.detail.contains("groupe akm") && !c.detail.contains("usermod"),
            "{}",
            c.detail
        );
        f.rssi_issue = None;
        f.helper_cap = false;
        assert!(!ok(&f).ok, "capability missing must fail");
        f.helper_cap = true;
        f.helper_exec = false;
        assert!(!ok(&f).ok);
    }

    fn by_id<'a>(c: &'a [Check], id: &str) -> &'a Check {
        c.iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("{id}"))
    }

    #[test]
    fn an_empty_system_fails_the_checks_that_need_installed_parts() {
        let f = Fake::new("empty");
        let c = run_checks(&f, true);
        let ids: Vec<&str> = c.iter().map(|x| x.id).collect();
        assert_eq!(
            ids,
            [
                "daemon",
                "bluetoothctl",
                "service",
                "dbus",
                "hidraw",
                "keymap",
                "udev",
                "fnmode",
                "rssi_helper"
            ],
            "the rows of the Diag tab, in its order"
        );
        assert!(by_id(&c, "daemon").ok && by_id(&c, "dbus").ok);
        assert!(by_id(&c, "daemon").detail.contains(akm_core::PKG_VERSION));
        for id in [
            "bluetoothctl",
            "service",
            "hidraw",
            "udev",
            "fnmode",
            "rssi_helper",
        ] {
            assert!(!by_id(&c, id).ok, "{id}");
        }
        assert!(by_id(&c, "bluetoothctl").detail.contains("NOT FOUND"));
        assert!(by_id(&c, "hidraw").detail.contains("no Apple hidraw"));
        assert!(by_id(&c, "keymap").ok);
        assert!(by_id(&c, "fnmode").detail.contains("unreadable"));
        let j = to_json(&c);
        assert_eq!(j["schema"], 1);
        assert_eq!(j["total"], 9);
        assert_eq!(j["passed"], 3);
        assert_eq!(j["checks"][0]["id"], "daemon");
        assert_eq!(j["checks"][4]["ok"], false);
    }

    #[test]
    fn a_complete_system_passes_everything() {
        let mut f = Fake::new("full");
        f.hidraw = Some("/dev/hidraw7".into());
        f.service_active = true;
        f.bluetoothctl = Run::Ok("bluetoothctl: 5.87".into());
        f.write("/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules", "");
        f.write("/usr/lib/apple-kb-monitor/rssi-helper", "");
        f.write("/sys/module/hid_apple/parameters/fnmode", "2\n");
        f.write(
            "/etc/modprobe.d/hid_apple.conf",
            "# comment\noptions hid_apple fnmode=2 iso_layout=0\n",
        );
        let c = run_checks(&f, true);
        assert!(c.iter().all(|x| x.ok), "{c:#?}");
        assert_eq!(
            by_id(&c, "bluetoothctl").detail,
            "bluetoothctl: 5.87",
            "R3: prefix once"
        );
        assert_eq!(by_id(&c, "hidraw").detail, "/dev/hidraw7: readable");
        assert!(by_id(&c, "fnmode").detail.contains("applied (sysfs) = 2"));
        assert_eq!(by_id(&c, "service").detail, "active (running)");
        assert_eq!(
            *f.runs.lock().unwrap(),
            [
                "bluetoothctl --version",
                "systemctl --user is-active apple-kb-monitord.service",
                "systemctl is-active --quiet keyd.service"
            ]
        );
    }

    #[test]
    fn mismatches_are_reported_with_the_way_out() {
        let mut f = Fake::new("mismatch");
        f.hidraw = Some("/dev/hidraw3".into());
        f.readable = Err("Permission denied (os error 13)".into());
        f.bluetoothctl = Run::Timeout;
        f.write("/sys/module/hid_apple/parameters/fnmode", "1\n");
        f.write("/etc/modprobe.d/a.conf", "options hid_apple fnmode=2\n");
        f.write("/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb", "garbage\n");
        let c = run_checks(&f, false);
        let h = by_id(&c, "hidraw");
        assert!(!h.ok && h.detail.contains("Permission denied") && h.detail.contains("uaccess"));
        let m = by_id(&c, "fnmode");
        assert!(!m.ok && m.detail.contains("differ"), "{}", m.detail);
        assert!(by_id(&c, "bluetoothctl")
            .detail
            .contains("no answer within 3 s"));
        assert!(!by_id(&c, "keymap").ok);
        assert!(!by_id(&c, "dbus").ok);
        std::fs::remove_file(f.path("/etc/modprobe.d/a.conf")).unwrap();
        let c = run_checks(&f, true);
        let m = by_id(&c, "fnmode");
        assert!(m.ok && m.detail.contains("not persistent"));
    }

    #[test]
    fn modprobe_options_are_parsed_like_the_window_does() {
        assert_eq!(parse_modprobe("options hid_apple fnmode=2\n"), Some(2));
        assert_eq!(
            parse_modprobe("options hid_apple iso_layout=0 fnmode=3 # x"),
            Some(3)
        );
        assert_eq!(parse_modprobe("# options hid_apple fnmode=1"), None);
        assert_eq!(parse_modprobe("options other fnmode=1"), None);
        assert_eq!(parse_modprobe("options hid_apple fnmode=9"), None);
        assert_eq!(
            parse_modprobe("options hid_apple fnmode=1\noptions hid_apple fnmode=2\n"),
            Some(2)
        );
    }

    #[test]
    fn a_program_that_hangs_is_killed_and_reported() {
        let t = std::time::Instant::now();
        let r = run_probe(Command::new("sleep").arg("30"), Duration::from_millis(150));
        assert_eq!(r, Run::Timeout);
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(
            run_probe(
                &mut Command::new("definitely-not-a-binary-akm"),
                COMMAND_TIMEOUT
            ),
            Run::NotFound
        );
        assert_eq!(
            run_probe(
                Command::new("sh").args(["-c", "echo one; echo two"]),
                COMMAND_TIMEOUT
            ),
            Run::Ok("one".into())
        );
        assert!(matches!(
            run_probe(Command::new("sh").args(["-c", "echo no >&2; exit 4"]), COMMAND_TIMEOUT),
            Run::Failed(4, e) if e == "no"
        ));
        assert_eq!(
            run_probe(
                Command::new("sh").args(["-c", "head -c 200000 /dev/zero; echo; echo done"]),
                COMMAND_TIMEOUT
            ),
            Run::Ok(String::from_utf8(vec![0; 200_000]).unwrap()),
            "an output larger than a pipe does not block the program"
        );
    }

    #[test]
    fn the_hidraw_node_is_never_opened() {
        let src = include_str!("diagnose.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(code.contains("libc::access"));
        for forbidden in ["File::open", "OpenOptions", "ioctl", "read_keyboard"] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
        assert!(SystemProbe.readable("/nonexistent/akm-node").is_err());
        assert!(SystemProbe.readable("/proc/self/status").is_ok());
    }

    #[test]
    fn versions_carry_the_pkgrel() {
        let src = include_str!("diagnose.rs");
        assert!(
            !src.contains(concat!("env!(\"CARGO", "_PKG_VERSION\")")),
            "R2/R12: use akm_core::PKG_VERSION"
        );
    }
}
