//! `akmctl doctor --fix`.

use akm_core::i18n::gettext;
use akm_core::tr;
use std::process::Command as Proc;

use crate::doctor::{self, Finding, Level, Report};
use crate::doctor_fix::{Fix, Request, ADAPTER_USB_ID, COMMAND};
use crate::{EXIT_ERROR, EXIT_OK};
use akm_core::paths::{HELPER, PKEXEC};

pub fn auto_fix(f: &Finding) -> Option<Fix> {
    let advice = f.fix.as_deref()?;
    match f.topic {
        "bluez-conf" => Some(Fix::BluezConf),
        "journal" if advice.starts_with("akmctl doctor --fix") => Some(Fix::BluezConf),
        "upower" => Some(Fix::UpowerConf),
        "adapter-pm"
            if f.id == "adapter-pm.active"
                && f.args.first().is_some_and(|a| a == ADAPTER_USB_ID) =>
        {
            Some(Fix::AdapterAutosuspend)
        }
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub fixes: Vec<Fix>,
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

pub fn request(fixes: &[Fix], restart: bool, dry_run: bool) -> Request {
    Request {
        fixes: fixes.to_vec(),
        restart: restart && fixes.iter().any(|f| f.service().is_some()),
        dry_run,
    }
}

pub trait Exec {
    fn helper(&self, privileged: bool, args: &[String]) -> Result<(), String>;
}

pub struct SystemExec;

impl Exec for SystemExec {
    fn helper(&self, privileged: bool, args: &[String]) -> Result<(), String> {
        if !std::path::Path::new(HELPER).exists() {
            return Err(tr!(
                "{HELPER} is not installed (package older than this akmctl?)",
                HELPER = HELPER
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
            .arg(COMMAND)
            .args(args)
            .status()
            .map_err(|e| tr!("cannot start the helper: {e}", e = e))?;
        if let Some(e) = crate::pkexec::failure(st.code(), HELPER).filter(|_| privileged) {
            return Err(e);
        }
        match st.code() {
            Some(0) => Ok(()),
            Some(c) => Err(tr!("{HELPER} failed (exit {c})", HELPER = HELPER, c = c)),
            None => Err(tr!("{HELPER} killed by a signal", HELPER = HELPER)),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub dry_run: bool,
    pub restart: bool,
    pub optional: bool,
}

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

pub fn apply(
    findings: &[Finding],
    o: Options,
    exec: &dyn Exec,
    say: &mut dyn FnMut(String),
) -> Outcome {
    let p = plan(findings, o.optional);
    for f in &p.optional {
        say(tr!(
            "optional, not applied (add --optional): {}",
            gettext(f.describe())
        ));
    }
    if p.fixes.is_empty() {
        say(tr!("nothing here can be corrected by the helper (see the \u{2192} lines above for the rest)"));
        return Outcome::Nothing;
    }
    say(if o.dry_run {
        tr!("corrections that --fix would apply:")
    } else {
        tr!("corrections:")
    });
    for f in &p.fixes {
        say(format!("  {:<20} {}", f.id(), gettext(f.describe())));
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
            println!("{}", tr!("\nafter the corrections:"));
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
        let f = check_usb_power(Some(&usb("8087:0026", "auto")), false);
        assert_eq!(f.level, Level::Warn);
        assert_eq!(auto_fix(&f), Some(Fix::AdapterAutosuspend));
        assert_eq!(
            auto_fix(&check_usb_power(Some(&usb("8087:0026", "auto")), true)),
            Some(Fix::AdapterAutosuspend)
        );
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
    fn autosuspend_advice_works_without_a_source_tree() {
        let ours = check_usb_power(Some(&usb("8087:0026", "auto")), false);
        assert_eq!(ours.fix.as_deref(), Some("akmctl doctor --fix"));
        let other = check_usb_power(Some(&usb("0a12:0001", "auto")), false);
        assert_eq!(other.level, Level::Info);
        let fix = other.fix.unwrap();
        assert!(
            fix.contains(r#"ATTR{idVendor}=="0a12", ATTR{idProduct}=="0001""#),
            "{fix}"
        );
        assert!(!fix.contains("udev/61-akm"), "{fix}");
        let odd = check_usb_power(Some(&usb("0a12:00'1", "auto")), false);
        assert_eq!(odd.fix, None);
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
        assert!(p.optional.is_empty(), "{:?}", p.optional);
        let mut ok = check_main_conf(&set_keys(BAD_CONF, Fix::BluezConf.wanted()));
        ok.push(check_upower("[UPower]\nNoPollBatteries=true\n"));
        ok.push(check_usb_power(Some(&usb("8087:0026", "on")), true));
        assert_eq!(plan(&ok, true), Plan::default());
    }

    #[test]
    fn what_the_helper_writes_is_what_the_diagnostic_wants() {
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
        let exec = Rec(RefCell::new(Vec::new()), Some("authentication dismissed"));
        let (out, _) = run(&broken(), Options::default(), &exec);
        assert_eq!(out, Outcome::Failed("authentication dismissed".into()));
    }
}
