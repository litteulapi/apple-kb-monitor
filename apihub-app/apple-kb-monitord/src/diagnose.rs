//! D-Bus `Diagnose()` (#120): the checks of the "Diag" tab of the window
//! (`apihub-app/src/diag_tab.rs`), run by the daemon and returned as JSON, so
//! the Plasma widget and the KDE module can show a diagnosis without the
//! window and without running programs themselves.
//!
//! ```json
//! {"schema":1,"daemon_version":"3.1.0","passed":8,"total":9,"checks":[
//!   {"id":"daemon","label":"Monitor daemon","ok":true,"detail":"..."}, ...]}
//! ```
//!
//! `id` is stable (clients may translate or attach a fix to it); `label` and
//! `detail` are in the daemon's language (French or English).
//!
//! Nothing here touches the keyboard: the hidraw node is looked up in sysfs
//! and tested with `access(2)`, never opened; the Fn mode is read in the
//! module's sysfs parameters. Every program run is bounded in time.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use akm_core::hid_params::Param;

use crate::notify::Lang;

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
    /// Run `bin args`, at most [`COMMAND_TIMEOUT`].
    fn run(&self, bin: &str, args: &[&str]) -> Run;
    /// A path of the system, under the probe's root.
    fn path(&self, absolute: &str) -> PathBuf;
    /// `/dev/hidrawN` of the Apple keyboard, from sysfs.
    fn apple_hidraw(&self) -> Option<String>;
    /// Can this user read `path`? (`access(2)`: the node is never opened.)
    fn readable(&self, path: &str) -> Result<(), String>;
    /// May this process run `path`? (`access(2)` with `X_OK`.)
    fn executable(&self, path: &str) -> bool;
    /// Why this daemon's last real RSSI read failed (#269), `None` after a
    /// success or before the first read.
    fn rssi_issue(&self) -> Option<akm_core::rssi::RssiIssue>;
}

/// The real system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProbe;

/// Run a program with a deadline; killed when it does not end in time.
pub fn run_bounded(cmd: &mut Command, timeout: Duration) -> Run {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return Run::NotFound;
    };
    let end = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                return Run::Timeout;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Run::Failed(-1, e.to_string()),
        }
    }
    match child.wait_with_output() {
        Ok(o) if o.status.success() => Run::Ok(
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
        ),
        Ok(o) => Run::Failed(
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        ),
        Err(e) => Run::Failed(-1, e.to_string()),
    }
}

impl Probe for SystemProbe {
    fn run(&self, bin: &str, args: &[&str]) -> Run {
        run_bounded(Command::new(bin).args(args), COMMAND_TIMEOUT)
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

/// `fnmode` set by the `options hid_apple ... fnmode=N` lines of one
/// modprobe.d file (last occurrence wins, comments ignored).
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

fn program(p: &dyn Probe, lang: Lang, id: &'static str, label: &str, bin: &str) -> Check {
    let (ok, detail) = match p.run(bin, &["--version"]) {
        Run::Ok(first) if first.is_empty() => (true, format!("{bin}: OK")),
        Run::Ok(first) => (true, format!("{bin}: {first}")),
        Run::Failed(_, err) if err.to_ascii_lowercase().contains("usage") => (
            true,
            format!("{bin}: {}", lang.t("installed", "install\u{e9}")),
        ),
        Run::Failed(code, _) => (false, format!("{bin}: exit {code}")),
        Run::Timeout => (
            false,
            match lang {
                Lang::En => format!(
                    "{bin}: no answer within {} s (killed)",
                    COMMAND_TIMEOUT.as_secs()
                ),
                Lang::Fr => format!(
                    "{bin} : pas de r\u{e9}ponse en {} s (arr\u{ea}t\u{e9})",
                    COMMAND_TIMEOUT.as_secs()
                ),
            },
        ),
        Run::NotFound => (
            false,
            format!("{bin}: {}", lang.t("NOT FOUND", "INTROUVABLE")),
        ),
    };
    Check {
        id,
        label: label.to_string(),
        ok,
        detail,
    }
}

fn file_check(
    p: &dyn Probe,
    lang: Lang,
    id: &'static str,
    label: &str,
    path: &str,
    present: &str,
) -> Check {
    let ok = p.path(path).exists();
    Check {
        id,
        label: label.to_string(),
        ok,
        detail: if ok {
            present.to_string()
        } else {
            format!("{path}: {}", lang.t("NOT FOUND", "INTROUVABLE"))
        },
    }
}

/// Every check, in the order of the Diag tab. `on_bus`: the daemon answers
/// on its D-Bus name (true when called through `Diagnose()`).
pub fn run_checks(p: &dyn Probe, lang: Lang, on_bus: bool) -> Vec<Check> {
    let mut out = Vec::new();
    // The daemon itself: it is the one answering.
    out.push(Check {
        id: "daemon",
        label: lang
            .t("Monitor daemon", "D\u{e9}mon de surveillance")
            .to_string(),
        ok: true,
        detail: format!("apple-kb-monitord {}", env!("CARGO_PKG_VERSION")),
    });
    out.push(program(
        p,
        lang,
        "bluetoothctl",
        lang.t("BlueZ CLI", "Outil BlueZ"),
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
            lang.t("active (running)", "actif (en cours)").into()
        } else {
            lang.t(
                "inactive / not found (daemon started by hand?)",
                "inactif / introuvable (d\u{e9}mon lanc\u{e9} \u{e0} la main ?)",
            )
            .into()
        },
    });
    out.push(Check {
        id: "dbus",
        label: format!("D-Bus {}", crate::service::BUS_NAME),
        ok: on_bus,
        detail: if on_bus {
            lang.t(
                "daemon reachable on the session bus",
                "d\u{e9}mon joignable sur le bus de session",
            )
            .into()
        } else {
            lang.t(
                "daemon without its session interface",
                "d\u{e9}mon sans son interface de session",
            )
            .into()
        },
    });
    // hidraw node of the keyboard: present AND readable (udev `uaccess`).
    let (hid_ok, hid_detail) = match p.apple_hidraw() {
        None => (
            false,
            lang.t(
                "no Apple hidraw device found (keyboard off or not paired?)",
                "aucun p\u{e9}riph\u{e9}rique hidraw Apple (clavier \u{e9}teint ou non appair\u{e9} ?)",
            )
            .to_string(),
        ),
        Some(path) => match p.readable(&path) {
            Ok(()) => (
                true,
                format!("{path}: {}", lang.t("readable", "lisible")),
            ),
            Err(e) => (
                false,
                format!(
                    "{path}: {e} \u{2014} {}",
                    lang.t(
                        "check the udev uaccess rule",
                        "v\u{e9}rifiez la r\u{e8}gle udev uaccess"
                    )
                ),
            ),
        },
    };
    out.push(Check {
        id: "hidraw",
        label: lang.t("hidraw readable", "hidraw lisible").into(),
        ok: hid_ok,
        detail: hid_detail,
    });
    // Key mapping (#247): udev hwdb written by `akmctl keymap apply`; none =
    // kernel mapping, the default. keyd is optional (#246).
    let hwdb = akm_core::keymap::HWDB_PATH;
    let (km_ok, mut km_detail) = match akm_core::keymap::read_installed(&p.path(hwdb)) {
        Ok(r) if r.is_empty() => (
            true,
            lang.t(
                "kernel mapping, no hwdb installed (optional: akmctl keymap)",
                "mappage du noyau, aucun hwdb install\u{e9} (optionnel : akmctl keymap)",
            )
            .to_string(),
        ),
        Ok(r) => (
            true,
            match lang {
                Lang::En => format!("{hwdb}: {} model(s) remapped", r.len()),
                Lang::Fr => format!("{hwdb} : {} mod\u{e8}le(s) remapp\u{e9}(s)", r.len()),
            },
        ),
        Err(e) => (
            false,
            format!(
                "{e} \u{2014} {}",
                lang.t(
                    "reinstall: akmctl keymap apply, or remove: akmctl keymap reset",
                    "r\u{e9}installer : akmctl keymap apply, ou retirer : akmctl keymap reset"
                )
            ),
        ),
    };
    let keyd_active = matches!(
        p.run("systemctl", &["is-active", "--quiet", "keyd.service"]),
        Run::Ok(_)
    );
    if keyd_active && p.path("/etc/keyd/apple-keyboard.conf").exists() {
        km_detail.push_str(lang.t(
            " \u{b7} keyd also active with /etc/keyd/apple-keyboard.conf (optional)",
            " \u{b7} keyd aussi actif avec /etc/keyd/apple-keyboard.conf (optionnel)",
        ));
    }
    out.push(Check {
        id: "keymap",
        label: lang.t("Key mapping", "Mappage des touches").into(),
        ok: km_ok,
        detail: km_detail,
    });
    out.push(file_check(
        p,
        lang,
        "udev",
        lang.t("udev rules", "R\u{e8}gles udev"),
        "/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules",
        lang.t(
            "70-apple-kb-hidraw.rules installed",
            "70-apple-kb-hidraw.rules install\u{e9}e",
        ),
    ));
    // hid_apple fnmode: the value applied (sysfs) against the configured one.
    let live = Param::FnMode.read_in(&p.path(akm_core::hid_params::SYSFS_DIR));
    let configured = configured_fnmode(&p.path("/etc/modprobe.d"));
    let show = |v: Option<i32>, none: &str| v.map_or(none.to_string(), |n| n.to_string());
    let state = match lang {
        Lang::En => format!(
            "applied (sysfs) = {} \u{b7} configured (modprobe.d) = {}",
            show(live, "unreadable (hid_apple not loaded?)"),
            show(configured, "none")
        ),
        Lang::Fr => format!(
            "appliqu\u{e9} (sysfs) = {} \u{b7} configur\u{e9} (modprobe.d) = {}",
            show(live, "illisible (hid_apple non charg\u{e9} ?)"),
            show(configured, "aucun")
        ),
    };
    let (fn_ok, fn_detail) = match (live, configured) {
        (Some(l), Some(c)) if l == c => (true, state),
        (Some(_), Some(_)) => (
            false,
            format!(
                "{state} \u{2014} {}",
                lang.t(
                    "differ: reload hid_apple or reboot",
                    "diff\u{e9}rents : rechargez hid_apple ou red\u{e9}marrez"
                )
            ),
        ),
        (Some(_), None) => (
            true,
            format!(
                "{state} ({})",
                lang.t(
                    "not persistent: akmctl set fnmode N --persist",
                    "non persistant : akmctl set fnmode N --persist"
                )
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
    out.push(rssi_check(p, lang));
    out
}

/// The signal (#269): OK only when this daemon can really measure it. The
/// last real read decides (the daemon's groups are not the caller's); before
/// any read, can this process run the helper at all (`root:akm 0750`).
fn rssi_check(p: &dyn Probe, lang: Lang) -> Check {
    use akm_core::rssi;
    const HELPER: &str = "/usr/lib/apple-kb-monitor/rssi-helper";
    let path = p.path(HELPER);
    let code = if !path.exists() {
        Some(rssi::CODE_HELPER_MISSING.to_string())
    } else if let Some(i) = p.rssi_issue() {
        Some(i.code)
    } else if !p.executable(&path.to_string_lossy()) {
        Some(rssi::classify(&rssi::RssiError::Denied, rssi::user_listed_in_akm).code)
    } else {
        None
    };
    let fr = lang == Lang::Fr;
    Check {
        id: "rssi_helper",
        label: lang.t("Signal measure", "Mesure du signal").into(),
        ok: code.is_none(),
        detail: match code {
            None => lang
                .t(
                    "rssi-helper runnable by the service (group akm)",
                    "rssi-helper utilisable par le service (groupe akm)",
                )
                .into(),
            Some(c) => {
                let (why, fix) = rssi::explain(&c, fr);
                format!("{why}. {} {fix}", lang.t("Fix:", "Correction\u{a0}:"))
            }
        },
    }
}

/// The JSON document of `Diagnose()`.
pub fn to_json(checks: &[Check]) -> serde_json::Value {
    serde_json::json!({
        "schema": SCHEMA,
        "daemon_version": env!("CARGO_PKG_VERSION"),
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

    /// A fixture tree and canned program results: no hardware, no program run.
    pub struct Fake {
        pub root: PathBuf,
        pub hidraw: Option<String>,
        pub readable: Result<(), String>,
        pub runs: Mutex<Vec<String>>,
        pub service_active: bool,
        pub bluetoothctl: Run,
        pub helper_exec: bool,
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
        fn executable(&self, _path: &str) -> bool {
            self.helper_exec
        }
        fn rssi_issue(&self) -> Option<akm_core::rssi::RssiIssue> {
            self.rssi_issue.clone()
        }
    }

    /// #269 / audit C8: a helper the service may not run, or a refused last
    /// read, is a failed check with its fix — not a green "installed".
    #[test]
    fn the_signal_check_follows_the_real_read() {
        use akm_core::rssi;
        let mut f = Fake::new("rssi");
        f.write("/usr/lib/apple-kb-monitor/rssi-helper", "");
        let ok = |f: &Fake| by_id(&run_checks(f, Lang::Fr, true), "rssi_helper").clone();
        assert!(ok(&f).ok, "{:?}", ok(&f));
        f.rssi_issue = Some(rssi::classify(&rssi::RssiError::Denied, || true));
        let c = ok(&f);
        assert!(!c.ok);
        assert!(c.detail.contains("redémarrez l'ordinateur"), "{}", c.detail);
        f.rssi_issue = Some(rssi::classify(&rssi::RssiError::Denied, || false));
        assert!(ok(&f).detail.contains("sudo usermod -aG akm $USER"));
        // Never read yet, but this process may not run the helper.
        f.rssi_issue = None;
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
        let c = run_checks(&f, Lang::En, true);
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
        assert!(by_id(&c, "daemon")
            .detail
            .contains(env!("CARGO_PKG_VERSION")));
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
        // No hwdb is the default (kernel mapping): fine.
        assert!(by_id(&c, "keymap").ok);
        assert!(by_id(&c, "fnmode").detail.contains("unreadable"));
        let j = to_json(&c);
        assert_eq!(j["schema"], 1);
        assert_eq!(j["total"], 9);
        assert_eq!(j["passed"], 3);
        assert_eq!(j["checks"][0]["id"], "daemon");
        assert!(j["checks"][4]["ok"] == false);
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
        let c = run_checks(&f, Lang::Fr, true);
        assert!(c.iter().all(|x| x.ok), "{c:#?}");
        assert_eq!(by_id(&c, "hidraw").detail, "/dev/hidraw7: lisible");
        assert!(by_id(&c, "fnmode")
            .detail
            .contains("appliqu\u{e9} (sysfs) = 2"));
        assert_eq!(by_id(&c, "service").detail, "actif (en cours)");
        // Programs run: the three bounded commands, nothing else.
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
        let c = run_checks(&f, Lang::En, false);
        let h = by_id(&c, "hidraw");
        assert!(!h.ok && h.detail.contains("Permission denied") && h.detail.contains("uaccess"));
        let m = by_id(&c, "fnmode");
        assert!(!m.ok && m.detail.contains("differ"), "{}", m.detail);
        assert!(by_id(&c, "bluetoothctl")
            .detail
            .contains("no answer within 3 s"));
        assert!(!by_id(&c, "keymap").ok);
        assert!(!by_id(&c, "dbus").ok);
        // Live value without a configured one: fine, said not persistent.
        std::fs::remove_file(f.path("/etc/modprobe.d/a.conf")).unwrap();
        let c = run_checks(&f, Lang::En, true);
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
        let t = Instant::now();
        let r = run_bounded(Command::new("sleep").arg("30"), Duration::from_millis(150));
        assert_eq!(r, Run::Timeout);
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(
            run_bounded(
                &mut Command::new("definitely-not-a-binary-akm"),
                COMMAND_TIMEOUT
            ),
            Run::NotFound
        );
        assert_eq!(
            run_bounded(
                Command::new("sh").args(["-c", "echo one; echo two"]),
                COMMAND_TIMEOUT
            ),
            Run::Ok("one".into())
        );
        assert!(matches!(
            run_bounded(Command::new("sh").args(["-c", "echo no >&2; exit 4"]), COMMAND_TIMEOUT),
            Run::Failed(4, e) if e == "no"
        ));
    }

    /// The node is tested with access(2), never opened: no HID traffic.
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
}
