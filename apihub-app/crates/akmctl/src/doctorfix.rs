//! `akmctl doctor --fix` (#106): has the corrections the diagnostic proposes
//! applied by the privileged helper `akm-doctor-fix`, through `pkexec`
//! (polkit `com.agenceapi.AppleKbMonitor.doctor-fix`, `auth_admin`).
//!
//! akmctl itself writes nothing and stays unprivileged: it only turns
//! findings into identifiers of the helper's closed list
//! ([`crate::doctor_fix::Fix`]). What the helper cannot do (pairing, trust,
//! disk space, the user's own services) stays a line of advice, as without
//! `--fix`.

use std::process::Command as Proc;

use crate::doctor::{self, Finding, Level, Report};
use crate::doctor_fix::{Fix, Request, ADAPTER_USB_ID, HELPER};
use crate::{EXIT_ERROR, EXIT_OK};

/// Absolute path: never resolved through `$PATH`.
const PKEXEC: &str = "/usr/bin/pkexec";

/// The correction of the helper's list that answers `f`, if there is one.
pub fn auto_fix(f: &Finding) -> Option<Fix> {
    let advice = f.fix.as_deref()?;
    match f.topic {
        "bluez-conf" => Some(Fix::BluezConf),
        // bluetoothd said at start that it ignored a key of main.conf
        "journal" if advice.contains("akm-conf.py bluez") => Some(Fix::BluezConf),
        "upower" => Some(Fix::UpowerConf),
        // the rule is written for one adapter; on another it would do nothing
        "adapter-pm" if f.text.starts_with(&format!("{ADAPTER_USB_ID} ")) => {
            Some(Fix::AdapterAutosuspend)
        }
        _ => None,
    }
}

/// What `--fix` will ask of the helper, and what it leaves aside.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Corrections of a problem (`[ !! ]` or `[ KO ]`), each once, in the
    /// order of the helper's list.
    pub fixes: Vec<Fix>,
    /// Corrections of a mere remark (`[info]`, "optional: ..."): only with
    /// `--optional`.
    pub optional: Vec<Fix>,
}

pub fn plan(findings: &[Finding], with_optional: bool) -> Plan {
    let mut p = Plan::default();
    for f in findings {
        let Some(fix) = auto_fix(f) else { continue };
        if f.level >= Level::Warn || with_optional {
            p.fixes.push(fix);
        } else if f.level == Level::Info {
            p.optional.push(fix);
        }
    }
    p.fixes.sort();
    p.fixes.dedup();
    p.optional.sort();
    p.optional.dedup();
    p.optional.retain(|f| !p.fixes.contains(f));
    p
}

/// The helper's request for `fixes`; `--restart` is dropped when none of
/// them has a service (the helper would refuse it).
pub fn request(fixes: &[Fix], restart: bool, dry_run: bool) -> Request {
    Request {
        fixes: fixes.to_vec(),
        restart: restart && fixes.iter().any(|f| f.service().is_some()),
        dry_run,
    }
}

/// Starts the helper. A dry run only reads world-readable files: it is run
/// directly, without `pkexec` and without a password.
pub trait Exec {
    fn helper(&self, privileged: bool, args: &[String]) -> Result<(), String>;
}

pub struct SystemExec;

impl Exec for SystemExec {
    fn helper(&self, privileged: bool, args: &[String]) -> Result<(), String> {
        if !std::path::Path::new(HELPER).exists() {
            return Err(format!(
                "{HELPER} is not installed (package older than this akmctl?)"
            ));
        }
        let mut cmd = if privileged {
            let mut c = Proc::new(PKEXEC);
            c.arg(HELPER);
            c
        } else {
            Proc::new(HELPER)
        };
        let st = cmd
            .args(args)
            .status()
            .map_err(|e| format!("cannot start the helper: {e}"))?;
        match st.code() {
            Some(0) => Ok(()),
            Some(126) if privileged => Err("authentication dismissed or not authorized".into()),
            Some(127) if privileged => Err("authentication failed or helper not found".into()),
            Some(c) => Err(format!("{HELPER} failed (exit {c})")),
            None => Err(format!("{HELPER} killed by a signal")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub dry_run: bool,
    pub restart: bool,
    pub optional: bool,
}

/// What happened, for the caller to decide what to print next.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No finding has a correction in the helper's list.
    Nothing,
    /// Dry run shown; nothing changed.
    DryRun,
    /// Corrections applied.
    Applied,
    Failed(String),
}

/// Prints the plan and runs the helper once for all the corrections (one
/// authentication dialog).
pub fn apply(
    findings: &[Finding],
    o: Options,
    exec: &dyn Exec,
    say: &mut dyn FnMut(String),
) -> Outcome {
    let p = plan(findings, o.optional);
    for f in &p.optional {
        say(format!(
            "optional, not applied (add --optional): {}",
            f.describe()
        ));
    }
    if p.fixes.is_empty() {
        say("nothing here can be corrected by the helper (see the \u{2192} lines above for the rest)".into());
        return Outcome::Nothing;
    }
    say(if o.dry_run {
        "corrections that --fix would apply:".into()
    } else {
        "corrections:".into()
    });
    for f in &p.fixes {
        say(format!("  {:<20} {}", f.id(), f.describe()));
    }
    let req = request(&p.fixes, o.restart, o.dry_run);
    match exec.helper(!o.dry_run, &req.args()) {
        Ok(()) if o.dry_run => Outcome::DryRun,
        Ok(()) => Outcome::Applied,
        Err(e) => Outcome::Failed(e),
    }
}

fn exit_of(r: &Report) -> u8 {
    if r.verdict.0 >= Level::Bad {
        EXIT_ERROR
    } else {
        EXIT_OK
    }
}

/// `akmctl doctor --fix [--dry-run] [--restart-services] [--optional]`.
pub fn command(mac: Option<&str>, o: Options) -> u8 {
    let before = doctor::gather(mac);
    print!("{}", doctor::to_text(&before));
    println!();
    match apply(&before.findings, o, &SystemExec, &mut |l| println!("{l}")) {
        Outcome::Nothing | Outcome::DryRun => exit_of(&before),
        Outcome::Failed(e) => {
            eprintln!("akmctl: {e}");
            EXIT_ERROR
        }
        Outcome::Applied => {
            let after = doctor::gather(mac);
            println!("\nafter the corrections:");
            print!("{}", doctor::to_text(&after));
            exit_of(&after)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::doctor::{
        check_main_conf, check_upower, check_usb_power, journal_findings, keyboard_findings,
        UsbPower,
    };
    use crate::doctor_fix::{parse_args, set_keys};
    use akm_core::recovery::JournalKind;

    const BAD_CONF: &str = "[General]\nExperimental = true\n#FastConnectable = false\n\n[Policy]\n#ReconnectAttempts=7\n\n[AdvMon]\nReconnectUUIDs=00001124-0000-1000-8000-00805f9b34fb\nReconnectAttempts=7\n";

    fn usb(id: &str, control: &str) -> UsbPower {
        UsbPower {
            id: id.into(),
            control: control.into(),
            runtime_status: "suspended".into(),
            suspended_ms: 430_000,
            delay_ms: "2000".into(),
        }
    }

    /// A simulated machine with every correctable problem at once.
    fn broken() -> Vec<Finding> {
        let mut v = check_main_conf(BAD_CONF);
        v.push(check_upower("[UPower]\nNoPollBatteries=false\n"));
        v.push(check_usb_power(Some(&usb("8087:0026", "auto")), false));
        v.extend(journal_findings(&[(
            JournalKind::ConfigIgnored,
            2,
            "12:00:00".into(),
        )]));
        v
    }

    #[derive(Default)]
    struct Rec(RefCell<Vec<(bool, Vec<String>)>>, Option<&'static str>);
    impl Exec for Rec {
        fn helper(&self, privileged: bool, args: &[String]) -> Result<(), String> {
            self.0.borrow_mut().push((privileged, args.to_vec()));
            self.1.map_or(Ok(()), |e| Err(e.to_string()))
        }
    }

    fn run(findings: &[Finding], o: Options, exec: &Rec) -> (Outcome, String) {
        let mut text = String::new();
        let out = apply(findings, o, exec, &mut |l| {
            text.push_str(&l);
            text.push('\n');
        });
        (out, text)
    }

    #[test]
    fn a_missing_udev_rule_is_ko_and_planned() {
        // #106: "test par environnement simulé (règle udev absente → KO)".
        let f = check_usb_power(Some(&usb("8087:0026", "auto")), false);
        assert_eq!(f.level, Level::Warn);
        assert_eq!(auto_fix(&f), Some(Fix::AdapterAutosuspend));
        // rule installed but not applied yet: the helper re-triggers udev
        assert_eq!(
            auto_fix(&check_usb_power(Some(&usb("8087:0026", "auto")), true)),
            Some(Fix::AdapterAutosuspend)
        );
        // healthy, not on USB, or another adapter (the rule would not match it)
        assert_eq!(
            auto_fix(&check_usb_power(Some(&usb("8087:0026", "on")), true)),
            None
        );
        assert_eq!(auto_fix(&check_usb_power(None, false)), None);
        assert_eq!(
            auto_fix(&check_usb_power(Some(&usb("0a12:0001", "auto")), false)),
            None
        );
    }

    #[test]
    fn the_plan_holds_each_correction_once_and_keeps_optional_ones_aside() {
        let p = plan(&broken(), false);
        assert_eq!(
            p.fixes,
            [Fix::BluezConf, Fix::AdapterAutosuspend],
            "3 findings ask for bluez-conf: once"
        );
        assert_eq!(p.optional, [Fix::UpowerConf]);
        let p = plan(&broken(), true);
        assert_eq!(
            p.fixes,
            [Fix::BluezConf, Fix::UpowerConf, Fix::AdapterAutosuspend]
        );
        assert!(p.optional.is_empty());
        // a healthy machine: nothing
        let mut ok = check_main_conf(&set_keys(BAD_CONF, Fix::BluezConf.wanted()));
        ok.push(check_upower("[UPower]\nNoPollBatteries=true\n"));
        ok.push(check_usb_power(Some(&usb("8087:0026", "on")), true));
        assert_eq!(plan(&ok, true), Plan::default());
    }

    #[test]
    fn what_the_helper_writes_is_what_the_diagnostic_wants() {
        // The file as the helper rewrites it passes the very check that failed.
        assert!(check_main_conf(BAD_CONF)
            .iter()
            .any(|f| f.level == Level::Bad));
        let fixed = set_keys(BAD_CONF, Fix::BluezConf.wanted());
        let v = check_main_conf(&fixed);
        assert!(
            !v.is_empty() && v.iter().all(|f| f.level == Level::Ok && f.fix.is_none()),
            "{v:?}"
        );
        let up = set_keys(
            "[UPower]\nNoPollBatteries=false\n",
            Fix::UpowerConf.wanted(),
        );
        assert_eq!(check_upower(&up).level, Level::Ok);
        // from no file at all
        assert!(check_main_conf(&set_keys("", Fix::BluezConf.wanted()))
            .iter()
            .all(|f| f.level == Level::Ok));
        assert_eq!(
            check_upower(&set_keys("", Fix::UpowerConf.wanted())).level,
            Level::Ok
        );
    }

    #[test]
    fn advice_that_is_not_a_root_correction_is_never_sent_to_the_helper() {
        // pairing, trust, adapter power, repair, daemon: not in the list
        let mut v = keyboard_findings(None, Some(false));
        v.extend(keyboard_findings(None, None));
        v.extend(journal_findings(&[
            (JournalKind::Auth, 1, "t".into()),
            (JournalKind::StorageError, 1, "t".into()),
            (JournalKind::GetReportTimeout, 1, "t".into()),
        ]));
        assert!(v.iter().any(|f| f.fix.is_some()));
        assert_eq!(plan(&v, true), Plan::default());
        let exec = Rec::default();
        let (out, text) = run(&v, Options::default(), &exec);
        assert_eq!(out, Outcome::Nothing);
        assert!(exec.0.borrow().is_empty(), "helper started for nothing");
        assert!(text.contains("nothing here can be corrected by the helper"));
    }

    #[test]
    fn fix_starts_the_helper_once_through_pkexec_with_arguments_it_accepts() {
        let exec = Rec::default();
        let (out, text) = run(&broken(), Options::default(), &exec);
        assert_eq!(out, Outcome::Applied);
        let calls = exec.0.borrow();
        assert_eq!(
            *calls,
            [(
                true,
                vec!["bluez-conf".to_string(), "adapter-autosuspend".to_string()]
            )]
        );
        assert!(parse_args(&calls[0].1).is_ok());
        assert!(
            text.contains("optional, not applied (add --optional): UPower.conf"),
            "{text}"
        );
        drop(calls);
        // --optional --restart-services
        let exec = Rec::default();
        let o = Options {
            dry_run: false,
            restart: true,
            optional: true,
        };
        assert_eq!(run(&broken(), o, &exec).0, Outcome::Applied);
        let calls = exec.0.borrow();
        assert_eq!(
            calls[0].1,
            [
                "bluez-conf",
                "upower-conf",
                "adapter-autosuspend",
                "--restart"
            ]
        );
        assert!(parse_args(&calls[0].1).is_ok());
    }

    #[test]
    fn dry_run_never_asks_for_privileges() {
        let exec = Rec::default();
        let o = Options {
            dry_run: true,
            restart: true,
            optional: false,
        };
        let (out, text) = run(&broken(), o, &exec);
        assert_eq!(out, Outcome::DryRun);
        assert_eq!(
            *exec.0.borrow(),
            [(
                false,
                vec![
                    "bluez-conf".to_string(),
                    "adapter-autosuspend".into(),
                    "--restart".into(),
                    "--dry-run".into()
                ]
            )]
        );
        assert!(text.contains("corrections that --fix would apply:"));
    }

    #[test]
    fn restart_is_dropped_when_no_correction_has_a_service() {
        let only_rule = [check_usb_power(Some(&usb("8087:0026", "auto")), false)];
        let exec = Rec::default();
        let o = Options {
            dry_run: false,
            restart: true,
            optional: false,
        };
        assert_eq!(run(&only_rule, o, &exec).0, Outcome::Applied);
        let calls = exec.0.borrow();
        assert_eq!(calls[0].1, ["adapter-autosuspend"]);
        assert!(
            parse_args(&calls[0].1).is_ok(),
            "the helper refuses --restart alone with the rule"
        );
    }

    #[test]
    fn a_refused_authentication_is_a_failure() {
        let exec = Rec(
            RefCell::new(Vec::new()),
            Some("authentication dismissed or not authorized"),
        );
        let (out, _) = run(&broken(), Options::default(), &exec);
        assert_eq!(
            out,
            Outcome::Failed("authentication dismissed or not authorized".into())
        );
    }
}
