//! `akm-helper` — the one privileged program, run through pkexec; `argv[1]` names the command
//! and the polkit action (`org.freedesktop.policykit.exec.argv1`).

use akm_helper::tr;
use std::path::Path;
use std::process::ExitCode;

use akm_helper::hid_params::Param;
use akm_helper::{fsutil, params};

mod cmd {

    pub mod doctor_fix;
    pub mod hid_control;
    pub mod hid_inspect;
    pub mod keymap_helper;
}

const SYSFS_DIR: &str = "/sys/module/hid_apple/parameters";
const MODPROBE_CONF: &str = "/etc/modprobe.d/hid_apple.conf";

use akm_helper::EX_USAGE;

#[derive(Debug, PartialEq, Eq)]
struct Request {
    set: Vec<(&'static str, i32)>,
    persist: bool,
}

const USAGE: &str = "usage: akm-helper set-fnmode <0-4>|NAME=VALUE... [--persist]";

/// Commands, one per polkit action; anything else is refused before any work.
const COMMANDS: &str = "set-fnmode | install-keymap | hid-control | hid-inspect | doctor-fix";

fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Request, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let (verb, rest) = a.split_first().ok_or(USAGE)?;
    let (rest, persist) = match rest.split_last() {
        Some((&"--persist", r)) => (r, true),
        _ => (rest, false),
    };
    if let Some(f) = rest.iter().find(|t| t.starts_with("--")) {
        return Err(tr!("unknown option {f}", f = format!("{:?}", f)));
    }
    if *verb != "set-fnmode" {
        return Err(tr!("unknown command {verb}", verb = format!("{:?}", verb)));
    }
    if let [val] = rest {
        if !val.contains('=') {
            let mode = match val.as_bytes() {
                [d @ b'0'..=b'9'] if Param::FnMode.valid(i32::from(d - b'0')) => {
                    i32::from(d - b'0')
                }
                _ => {
                    return Err(tr!(
                        "invalid fnmode {val} (expected 0, 1, 2, 3 or 4)",
                        val = format!("{:?}", val)
                    ))
                }
            };
            return Ok(Request {
                set: vec![("fnmode", mode)],
                persist,
            });
        }
    }
    if rest.is_empty() || rest.len() > akm_helper::keymap::KERNEL_PARAMS.len() {
        return Err(USAGE.into());
    }
    let mut set: Vec<(&'static str, i32)> = Vec::new();
    for t in rest {
        let (n, v) = params::parse_assignment(t)?;
        if set.iter().any(|(m, _)| *m == n) {
            return Err(tr!("parameter {n} given twice", n = n));
        }
        set.push((n, v));
    }
    Ok(Request { set, persist })
}

#[cfg(test)]
fn merge_fnmode(existing: &str, mode: u8) -> String {
    params::merge(existing, &[("fnmode", i32::from(mode))])
}

#[cfg(test)]
fn persist(path: &Path, mode: u8) -> std::io::Result<()> {
    params::persist(path, &[("fnmode", i32::from(mode))])
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Applied,
    /// Persisted; `hid_apple` not loaded, applied at next module load.
    PersistedOnly,
    Failed(String),
}

fn run(req: &Request, sysfs_dir: &Path, conf: &Path) -> Outcome {
    if req.persist {
        if let Err(e) = params::persist(conf, &req.set) {
            return Outcome::Failed(tr!("{path} not updated: {e}", path = conf.display(), e = e));
        }
    }
    for &(name, v) in &req.set {
        match params::write_sysfs(sysfs_dir, name, v) {
            Ok(()) => {}
            Err(e)
                if req.persist
                    && e.kind() == std::io::ErrorKind::NotFound
                    && !sysfs_dir.exists() =>
            {
                return Outcome::PersistedOnly
            }
            Err(e) if req.persist => {
                return Outcome::Failed(tr!(
                    "{path} updated but cannot write {dir}/{name}: {e}",
                    path = conf.display(),
                    dir = sysfs_dir.display(),
                    name = name,
                    e = e
                ))
            }
            Err(e) => {
                return Outcome::Failed(tr!(
                    "cannot write {dir}/{name}: {e}",
                    dir = sysfs_dir.display(),
                    name = name,
                    e = e
                ))
            }
        }
    }
    Outcome::Applied
}

/// Strict dispatch on `argv[1]`: the polkit action was chosen from it.
fn dispatch(args: &[String]) -> ExitCode {
    let Some((verb, rest)) = args.split_first() else {
        eprintln!("akm-helper: usage: akm-helper {COMMANDS} ...");
        return ExitCode::from(EX_USAGE);
    };
    match verb.as_str() {
        "set-fnmode" => set_fnmode(args),
        "install-keymap" => cmd::keymap_helper::run(rest),
        "hid-control" => cmd::hid_control::run(rest),
        "hid-inspect" => cmd::hid_inspect::run(rest),
        "doctor-fix" => cmd::doctor_fix::run(rest),
        v => {
            eprintln!(
                "akm-helper: {}",
                tr!(
                    "unknown command {verb} (expected {list})",
                    verb = format!("{v:?}"),
                    list = COMMANDS
                )
            );
            ExitCode::from(EX_USAGE)
        }
    }
}

fn main() -> ExitCode {
    fsutil::lock_umask();
    akm_helper::i18n::init_privileged();
    match utf8_args(std::env::args_os().skip(1)) {
        Ok(args) => dispatch(&args),
        Err(e) => {
            eprintln!("akm-helper: {e}");
            ExitCode::from(EX_USAGE)
        }
    }
}

/// argv as UTF-8, or the usage error for the first argument that is not (never a panic).
fn utf8_args(args: impl Iterator<Item = std::ffi::OsString>) -> Result<Vec<String>, String> {
    args.enumerate()
        .map(|(i, a)| {
            a.into_string().map_err(|a| match i {
                0 => tr!(
                    "unknown command {verb} (expected {list})",
                    verb = format!("{a:?}"),
                    list = COMMANDS
                ),
                _ => tr!("unknown option {f}", f = format!("{a:?}")),
            })
        })
        .collect()
}

fn set_fnmode(args: &[String]) -> ExitCode {
    let req = match parse_args(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akm-helper: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    match run(&req, Path::new(SYSFS_DIR), Path::new(MODPROBE_CONF)) {
        Outcome::Applied => ExitCode::SUCCESS,
        Outcome::PersistedOnly => {
            let s: Vec<String> = req.set.iter().map(|(n, v)| format!("{n}={v}")).collect();
            eprintln!(
                "akm-helper: {}",
                tr!(
                    "hid_apple not loaded: {params} saved, applied at next module load",
                    params = s.join(" ")
                )
            );
            ExitCode::SUCCESS
        }
        Outcome::Failed(m) => {
            eprintln!("akm-helper: {m}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        for n in 0..=3u8 {
            assert_eq!(
                parse_args(&["set-fnmode", &n.to_string()]),
                Ok(Request {
                    set: vec![("fnmode", i32::from(n))],
                    persist: false
                })
            );
        }
        assert_eq!(
            parse_args(&["set-fnmode", "2", "--persist"]),
            Ok(Request {
                set: vec![("fnmode", 2)],
                persist: true
            })
        );
    }

    #[test]
    fn rejects_invalid_values() {
        for v in [
            "5", "9", "10", "-1", "+1", "01", "", " 1", "1\n", "a", "1;", "١", "0x1", "../x",
        ] {
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
        assert!(parse_args(&["set-params", "fnmode=1"]).is_err());
        assert!(parse_args(&["set-fnmode", "1", "fnmode=2"]).is_err());
    }

    #[test]
    fn merge_creates_and_replaces() {
        assert_eq!(merge_fnmode("", 2), "options hid_apple fnmode=2\n");
        assert_eq!(
            merge_fnmode("options hid_apple fnmode=1\n", 3),
            "options hid_apple fnmode=3\n"
        );
        assert_eq!(
            merge_fnmode(
                "# c\noptions hid_apple iso_layout=0 fnmode=1 swap_opt_cmd=1\noptions foo a=1\n",
                2
            ),
            "# c\noptions hid_apple iso_layout=0 fnmode=2 swap_opt_cmd=1\noptions foo a=1\n"
        );
        assert_eq!(
            merge_fnmode("options hid_apple iso_layout=1\n", 0),
            "options hid_apple iso_layout=1 fnmode=0\n"
        );
        assert_eq!(
            merge_fnmode("# only comment\n", 1),
            "# only comment\noptions hid_apple fnmode=1\n"
        );
    }

    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::Mutex;

    static UMASK: Mutex<()> = Mutex::new(());

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("akm-helper-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn perm(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    fn persist_legacy(path: &Path, mode: u8) -> std::io::Result<()> {
        let new = merge_fnmode("", mode);
        let tmp = path.with_extension("conf.akm-tmp");
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(new.as_bytes())?;
        fs::rename(&tmp, path)
    }

    #[test]
    fn umask_000_old_defect_then_fix() {
        let _g = UMASK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = tmpdir("umask");
        let old = fsutil::set_umask(0);
        let legacy = d.join("legacy.conf");
        persist_legacy(&legacy, 1).unwrap();
        let fixed = d.join("hid_apple.conf");
        persist(&fixed, 1).unwrap();
        let _ = fsutil::set_umask(0o077);
        let strict = d.join("strict.conf");
        persist(&strict, 1).unwrap();
        let _ = fsutil::set_umask(old);
        assert_eq!(
            perm(&legacy),
            0o666,
            "old code: world-writable under umask 000"
        );
        assert_eq!(perm(&fixed), 0o644, "umask 000 must still give 0644");
        assert_eq!(perm(&strict), 0o644, "umask 077 must still give 0644");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn existing_world_writable_file_is_tightened() {
        let d = tmpdir("tighten");
        let p = d.join("hid_apple.conf");
        fs::write(&p, "options hid_apple fnmode=1\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
        persist(&p, 2).unwrap();
        assert_eq!(perm(&p), 0o644);
        assert!(!d.join("hid_apple.conf.akm-tmp").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn refuses_symlink_destination() {
        let d = tmpdir("symlink");
        let target = d.join("victim");
        fs::write(&target, "keep\n").unwrap();
        let p = d.join("hid_apple.conf");
        symlink(&target, &p).unwrap();
        assert!(persist(&p, 2).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep\n");
        assert!(fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn stale_symlink_tmp_is_not_followed() {
        let d = tmpdir("staletmp");
        let target = d.join("victim");
        fs::write(&target, "keep\n").unwrap();
        symlink(&target, d.join("hid_apple.conf.akm-tmp")).unwrap();
        let p = d.join("hid_apple.conf");
        persist(&p, 3).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep\n");
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "options hid_apple fnmode=3\n"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn persist_roundtrip_in_tempdir() {
        let d = tmpdir("rt");
        let p = d.join("hid_apple.conf");
        persist(&p, 2).unwrap();
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "options hid_apple fnmode=2\n"
        );
        persist(&p, 0).unwrap();
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "options hid_apple fnmode=0\n"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn persist_without_module_loaded_still_persists() {
        let d = tmpdir("nomod");
        let conf = d.join("hid_apple.conf");
        let sysfs = d.join("absent");
        let req = Request {
            set: vec![("fnmode", 2)],
            persist: true,
        };
        assert_eq!(run(&req, &sysfs, &conf), Outcome::PersistedOnly);
        assert_eq!(
            fs::read_to_string(&conf).unwrap(),
            "options hid_apple fnmode=2\n"
        );
        let conf2 = d.join("other.conf");
        let req = Request {
            set: vec![("fnmode", 2)],
            persist: false,
        };
        assert!(matches!(run(&req, &sysfs, &conf2), Outcome::Failed(_)));
        assert!(!conf2.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn persist_with_module_loaded_writes_both() {
        let d = tmpdir("both");
        let conf = d.join("hid_apple.conf");
        let sysfs = d.join("params");
        fs::create_dir_all(&sysfs).unwrap();
        fs::write(sysfs.join("fnmode"), "1\n").unwrap();
        let req = Request {
            set: vec![("fnmode", 3)],
            persist: true,
        };
        assert_eq!(run(&req, &sysfs, &conf), Outcome::Applied);
        assert_eq!(fs::read_to_string(sysfs.join("fnmode")).unwrap(), "3\n");
        assert_eq!(
            fs::read_to_string(&conf).unwrap(),
            "options hid_apple fnmode=3\n"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn set_params_whitelist() {
        assert_eq!(
            parse_args(&["set-fnmode", "swap_opt_cmd=1", "iso_layout=-1", "--persist"]),
            Ok(Request {
                set: vec![("swap_opt_cmd", 1), ("iso_layout", -1)],
                persist: true
            })
        );
        assert_eq!(
            parse_args(&["set-fnmode", "fnmode=4"]),
            Ok(Request {
                set: vec![("fnmode", 4)],
                persist: false
            })
        );
        for bad in [
            &["set-fnmode"][..],
            &["set-fnmode", "fnmode=5"],
            &["set-fnmode", "fnmode=01"],
            &["set-fnmode", "fnmode= 1"],
            &["set-fnmode", "fnmode=1\n"],
            &["set-fnmode", "fnmode"],
            &["set-fnmode", "rightalt_as_rightctrl=1"],
            &["set-fnmode", "ejectcd_as_delete=1"],
            &["set-fnmode", "../../etc/passwd=1"],
            &["set-fnmode", "fnmode=1", "fnmode=2"],
            &["set-fnmode", "fnmode=1", "--force"],
            &["set-fnmode", "--persist", "fnmode=1"],
            &["set-fnmode", "a=1", "b=1", "c=1", "d=1", "e=1", "f=1"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn set_params_writes_each_parameter_and_persists() {
        let d = tmpdir("params");
        let sysfs = d.join("parameters");
        fs::create_dir_all(&sysfs).unwrap();
        for n in ["fnmode", "swap_opt_cmd"] {
            fs::write(sysfs.join(n), "0\n").unwrap();
        }
        let conf = d.join("hid_apple.conf");
        fs::write(&conf, "# keep\noptions hid_apple fnmode=1 iso_layout=0\n").unwrap();
        let req = parse_args(&["set-fnmode", "fnmode=2", "swap_opt_cmd=1", "--persist"]).unwrap();
        assert_eq!(run(&req, &sysfs, &conf), Outcome::Applied);
        assert_eq!(fs::read_to_string(sysfs.join("fnmode")).unwrap(), "2\n");
        assert_eq!(
            fs::read_to_string(sysfs.join("swap_opt_cmd")).unwrap(),
            "1\n"
        );
        assert_eq!(
            fs::read_to_string(&conf).unwrap(),
            "# keep\noptions hid_apple fnmode=2 iso_layout=0 swap_opt_cmd=1\n"
        );
        let req = parse_args(&["set-fnmode", "swap_ctrl_cmd=1"]).unwrap();
        assert!(matches!(run(&req, &sysfs, &conf), Outcome::Failed(_)));
        // a symlink in place of a parameter is never followed
        let victim = d.join("victim");
        fs::write(&victim, "keep\n").unwrap();
        symlink(&victim, sysfs.join("swap_fn_leftctrl")).unwrap();
        let req = parse_args(&["set-fnmode", "swap_fn_leftctrl=1"]).unwrap();
        assert!(matches!(run(&req, &sysfs, &conf), Outcome::Failed(_)));
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep\n");
        fs::remove_dir_all(&d).unwrap();
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    #[test]
    fn non_utf8_arguments_are_a_usage_error() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let bad = || OsString::from_vec(vec![0xff]);
        let e = utf8_args([bad()].into_iter()).unwrap_err();
        assert!(e.starts_with("unknown command \"\\xFF\""), "{e}");
        let e = utf8_args(
            [
                OsString::from("hid-inspect"),
                OsString::from("--mac"),
                bad(),
            ]
            .into_iter(),
        );
        assert!(e.unwrap_err().starts_with("unknown option"));
        let ok = utf8_args([OsString::from("hid-inspect")].into_iter());
        assert_eq!(ok.unwrap(), ["hid-inspect"]);
    }

    #[test]
    fn only_argv1_selects_the_command() {
        let v = |a: &[&str]| {
            a.iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
        };
        for bad in [
            &[][..],
            &["--persist", "set-fnmode", "1"],
            &["set-params", "fnmode=1"],
            &["/usr/lib/apple-kb-monitor/akm-helper", "hid-inspect"],
            &["HID-INSPECT"],
            &["hid-inspect\0"],
            &["inspect"],
            &["suspend"],
            &[""],
        ] {
            assert_eq!(dispatch(&v(bad)), ExitCode::from(EX_USAGE), "{bad:?}");
        }
        assert_eq!(
            dispatch(&v(&["hid-inspect", "suspend"])),
            ExitCode::from(EX_USAGE)
        );
        assert_eq!(
            dispatch(&v(&["hid-control", "--mac"])),
            ExitCode::from(EX_USAGE)
        );
        assert_eq!(
            dispatch(&v(&["doctor-fix", "/etc/passwd"])),
            ExitCode::from(EX_USAGE)
        );
        assert_eq!(
            dispatch(&v(&["install-keymap", "../x"])),
            ExitCode::from(EX_USAGE)
        );
    }
}
