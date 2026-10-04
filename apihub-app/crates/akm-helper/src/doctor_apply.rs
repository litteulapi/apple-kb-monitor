//! Applies the corrections of [`crate::doctor_fix`].
use crate::tr;
use std::path::{Path, PathBuf};

use crate::doctor_fix::{changes, set_keys, Fix, Request, ADAPTER_RULE, ADAPTER_RULE_PATH};
use crate::fsutil;
use crate::keymap_install::{Runner, UDEVADM};

pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
const MAX_CONF: u64 = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Paths {
    pub bluez: PathBuf,
    pub upower: PathBuf,
    pub rule: PathBuf,
}

impl Paths {
    #[must_use]
    pub fn system() -> Paths {
        Paths {
            bluez: PathBuf::from("/etc/bluetooth/main.conf"),
            upower: PathBuf::from("/etc/UPower/UPower.conf"),
            rule: PathBuf::from(ADAPTER_RULE_PATH),
        }
    }

    fn of(&self, fix: Fix) -> &Path {
        match fix {
            Fix::BluezConf => &self.bluez,
            Fix::UpowerConf => &self.upower,
            Fix::AdapterAutosuspend => &self.rule,
        }
    }
}

#[must_use]
pub fn backup_of(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".akm-bak");
    path.with_file_name(name)
}

fn commands(fix: Fix, restart: bool) -> Vec<Vec<&'static str>> {
    match fix {
        Fix::AdapterAutosuspend => vec![
            vec![UDEVADM, "control", "--reload"],
            vec![
                UDEVADM,
                "trigger",
                "--action=add",
                "--subsystem-match=usb",
                "--attr-match=idVendor=8087",
                "--attr-match=idProduct=0026",
            ],
        ],
        _ => match fix.service() {
            Some(unit) if restart => vec![vec![SYSTEMCTL, "restart", unit]],
            _ => Vec::new(),
        },
    }
}

fn one(
    fix: Fix,
    req: &Request,
    p: &Paths,
    r: &dyn Runner,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let path = p.of(fix);
    let old = fsutil::read_regular(path, MAX_CONF).map_err(|e| e.to_string())?;
    let new = match fix {
        Fix::AdapterAutosuspend => ADAPTER_RULE.to_string(),
        _ => set_keys(old.as_deref().unwrap_or(""), fix.wanted()),
    };
    let changed = old.as_deref() != Some(new.as_str());
    let cmds = commands(fix, req.restart);
    if req.dry_run {
        if changed {
            out.push(tr!("{}: would rewrite {}", fix.id(), path.display()));
            out.extend(
                changes(old.as_deref().unwrap_or(""), &new)
                    .into_iter()
                    .map(|l| format!("  {l}")),
            );
            if old.is_some() {
                out.push(tr!(
                    "  previous content would be kept as {}",
                    backup_of(path).display()
                ));
            }
        } else {
            out.push(tr!(
                "{}: {} already in place, nothing to write",
                fix.id(),
                path.display()
            ));
        }
        for c in &cmds {
            out.push(tr!("  would run: {}", c.join(" ")));
        }
        return Ok(());
    }
    if changed {
        if let Some(prev) = &old {
            fsutil::atomic_write(&backup_of(path), prev.as_bytes()).map_err(|e| e.to_string())?;
        }
        fsutil::atomic_write(path, new.as_bytes()).map_err(|e| e.to_string())?;
        out.push(match &old {
            Some(_) => tr!(
                "{}: {} rewritten; previous content kept as {}",
                fix.id(),
                path.display(),
                backup_of(path).display()
            ),
            None => tr!("{}: {} created", fix.id(), path.display()),
        });
    } else {
        out.push(tr!(
            "{}: {} already in place, nothing written",
            fix.id(),
            path.display()
        ));
    }
    for c in &cmds {
        r.run(c)?;
        out.push(tr!("  ran: {}", c.join(" ")));
    }
    if let (Some(unit), false, true) = (fix.service(), req.restart, changed) {
        out.push(tr!("  read by {unit} at its next start (reboot, or akmctl doctor --fix --restart-services)", unit = unit));
    }
    Ok(())
}

pub fn run(req: &Request, p: &Paths, r: &dyn Runner) -> (Vec<String>, Result<(), String>) {
    let mut out = Vec::new();
    for &fix in &req.fixes {
        if let Err(e) = one(fix, req, p, r, &mut out) {
            return (out, Err(format!("{}: {e}", fix.id())));
        }
    }
    (out, Ok(()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{backup_of, run, Paths, SYSTEMCTL};
    use crate::doctor_fix::{parse_args, Fix, Request, ADAPTER_RULE};
    use crate::keymap_install::{Runner, UDEVADM};

    #[derive(Default)]
    struct Rec(RefCell<Vec<String>>, bool);
    impl Runner for Rec {
        fn run(&self, argv: &[&str]) -> Result<(), String> {
            self.0.borrow_mut().push(argv.join(" "));
            if self.1 {
                Err("boom".into())
            } else {
                Ok(())
            }
        }
    }

    fn setup(tag: &str) -> (PathBuf, Paths) {
        let d = std::env::temp_dir().join(format!("akm-doctor-fix-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for sub in ["bluetooth", "UPower", "udev/rules.d"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        let p = Paths {
            bluez: d.join("bluetooth/main.conf"),
            upower: d.join("UPower/UPower.conf"),
            rule: d.join("udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules"),
        };
        (d, p)
    }

    fn req(a: &[&str]) -> Request {
        parse_args(a).unwrap()
    }

    fn snapshot(d: &Path) -> Vec<(PathBuf, String)> {
        let mut v = Vec::new();
        let mut todo = vec![d.to_path_buf()];
        while let Some(dir) = todo.pop() {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    todo.push(p);
                } else {
                    v.push((p.clone(), fs::read_to_string(&p).unwrap()));
                }
            }
        }
        v.sort();
        v
    }

    const STOCK: &str = "[General]\n#FastConnectable = false\n[AdvMon]\nReconnectAttempts = 3\n";

    #[test]
    fn bluez_conf_is_fixed_backed_up_and_the_second_run_changes_nothing() {
        let (d, p) = setup("bluez");
        fs::write(&p.bluez, STOCK).unwrap();
        let r = Rec::default();
        let (out, res) = run(&req(&["bluez-conf"]), &p, &r);
        assert_eq!(res, Ok(()), "{out:?}");
        let fixed = fs::read_to_string(&p.bluez).unwrap();
        assert!(
            fixed.contains("[General]\nFastConnectable = true\n"),
            "{fixed}"
        );
        assert!(
            fixed.contains(
                "[Policy]\nReconnectIntervals = 1,2,4,8,16,32,64\nReconnectAttempts = 7\n"
            ),
            "{fixed}"
        );
        assert!(!fixed.contains("ReconnectAttempts = 3"));
        assert_eq!(
            fs::read_to_string(backup_of(&p.bluez)).unwrap(),
            STOCK,
            "the file as it was"
        );
        assert!(
            r.0.borrow().is_empty(),
            "no service restarted without --restart"
        );
        assert!(
            out.iter()
                .any(|l| l.contains("bluetooth.service at its next start")),
            "{out:?}"
        );
        let before = snapshot(&d);
        let (out, res) = run(&req(&["bluez-conf"]), &p, &r);
        assert_eq!(res, Ok(()));
        assert!(
            out[0].contains("already in place, nothing written"),
            "{out:?}"
        );
        assert_eq!(snapshot(&d), before);
        assert_eq!(fs::read_to_string(backup_of(&p.bluez)).unwrap(), STOCK);
        assert_eq!(before.len(), 2, "{before:?}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dry_run_writes_nothing_and_runs_nothing() {
        let (d, p) = setup("dry");
        fs::write(&p.bluez, STOCK).unwrap();
        fs::write(&p.upower, "[UPower]\nNoPollBatteries=false\n").unwrap();
        let before = snapshot(&d);
        let r = Rec::default();
        let (out, res) = run(
            &req(&[
                "bluez-conf",
                "upower-conf",
                "adapter-autosuspend",
                "--restart",
                "--dry-run",
            ]),
            &p,
            &r,
        );
        assert_eq!(res, Ok(()));
        assert_eq!(snapshot(&d), before, "dry run changed a file");
        assert!(r.0.borrow().is_empty(), "dry run ran {:?}", r.0.borrow());
        let text = out.join("\n");
        for needle in [
            "bluez-conf: would rewrite",
            "  - ReconnectAttempts = 3",
            "  + FastConnectable = true",
            "  + ReconnectAttempts = 7",
            "main.conf.akm-bak",
            "upower-conf: would rewrite",
            "  - NoPollBatteries=false",
            "  + NoPollBatteries = true",
            "adapter-autosuspend: would rewrite",
            "would run: /usr/bin/systemctl restart bluetooth.service",
            "would run: /usr/bin/systemctl restart upower.service",
            "would run: /usr/bin/udevadm control --reload",
        ] {
            assert!(text.contains(needle), "{needle:?} missing in:\n{text}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn restart_runs_exactly_the_fixed_commands() {
        let (d, p) = setup("restart");
        let r = Rec::default();
        let (out, res) = run(
            &req(&[
                "bluez-conf",
                "upower-conf",
                "adapter-autosuspend",
                "--restart",
            ]),
            &p,
            &r,
        );
        assert_eq!(res, Ok(()), "{out:?}");
        assert_eq!(
            *r.0.borrow(),
            [
                format!("{SYSTEMCTL} restart bluetooth.service"),
                format!("{SYSTEMCTL} restart upower.service"),
                format!("{UDEVADM} control --reload"),
                format!("{UDEVADM} trigger --action=add --subsystem-match=usb --attr-match=idVendor=8087 --attr-match=idProduct=0026"),
            ]
        );
        assert_eq!(
            fs::read_to_string(&p.upower).unwrap(),
            "[UPower]\nNoPollBatteries = true\n"
        );
        assert_eq!(fs::read_to_string(&p.rule).unwrap(), ADAPTER_RULE);
        assert!(
            !backup_of(&p.upower).exists()
                && !backup_of(&p.rule).exists()
                && !backup_of(&p.bluez).exists()
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_udev_rule_is_installed_once_and_a_foreign_one_is_kept_as_backup() {
        let (d, p) = setup("rule");
        fs::write(&p.rule, "# hand-made\n").unwrap();
        let r = Rec::default();
        assert_eq!(run(&req(&["adapter-autosuspend"]), &p, &r).1, Ok(()));
        assert_eq!(fs::read_to_string(&p.rule).unwrap(), ADAPTER_RULE);
        assert_eq!(
            fs::read_to_string(backup_of(&p.rule)).unwrap(),
            "# hand-made\n"
        );
        assert!(
            backup_of(&p.rule).extension().is_some_and(|e| e != "rules"),
            "udev must not load the backup"
        );
        let before = snapshot(&d);
        let (out, res) = run(&req(&["adapter-autosuspend"]), &p, &r);
        assert_eq!(res, Ok(()));
        assert!(out[0].contains("already in place"));
        assert_eq!(snapshot(&d), before);
        assert_eq!(r.0.borrow().len(), 4);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_symlink_or_a_failing_command_stops_everything_after_it() {
        use std::os::unix::fs::symlink;
        let (d, p) = setup("refuse");
        let secret = d.join("secret");
        fs::write(&secret, "do not touch\n").unwrap();
        symlink(&secret, &p.bluez).unwrap();
        let r = Rec::default();
        let (out, res) = run(&req(&["bluez-conf", "upower-conf"]), &p, &r);
        assert!(res.unwrap_err().starts_with("bluez-conf: "), "{out:?}");
        assert_eq!(fs::read_to_string(&secret).unwrap(), "do not touch\n");
        assert!(!p.upower.exists(), "nothing after the failure");
        assert!(run(&req(&["bluez-conf", "--dry-run"]), &p, &r).1.is_err());
        let failing = Rec(RefCell::new(Vec::new()), true);
        let (_, res) = run(&req(&["adapter-autosuspend", "upower-conf"]), &p, &failing);
        assert_eq!(res, Err("adapter-autosuspend: boom".into()));
        assert!(!p.upower.exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_directory_or_an_oversized_file_is_an_error_not_a_creation() {
        let (d, p) = setup("missing");
        fs::remove_dir_all(d.join("UPower")).unwrap();
        let r = Rec::default();
        assert!(run(&req(&["upower-conf"]), &p, &r).1.is_err());
        assert!(
            !d.join("UPower").exists(),
            "the helper creates no directory"
        );
        fs::write(&p.bluez, "#".repeat(300 * 1024)).unwrap();
        assert!(run(&req(&["bluez-conf"]), &p, &r).1.is_err());
        assert!(!backup_of(&p.bluez).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn every_correction_of_the_list_has_a_file() {
        let p = Paths::system();
        assert_eq!(p.bluez.to_str(), Some("/etc/bluetooth/main.conf"));
        assert_eq!(p.upower.to_str(), Some("/etc/UPower/UPower.conf"));
        assert_eq!(
            p.rule.to_str(),
            Some("/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules")
        );
        assert_eq!(Fix::ALL.len(), 3);
    }
}
