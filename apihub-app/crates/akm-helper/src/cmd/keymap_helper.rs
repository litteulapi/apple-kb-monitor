//! `akm-helper install-keymap` — installs the udev hwdb file of the manual key mapping.

use std::process::ExitCode;

use akm_helper::keymap_install::{parse_args, parse_uid, run as apply, Paths, SystemRunner};

use akm_helper::{tr, EX_USAGE};

pub fn run(argv: &[String]) -> ExitCode {
    let verb = match parse_args(argv) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("akm-helper install-keymap: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let uid = match std::env::var("PKEXEC_UID")
        .map_err(|_| tr!("PKEXEC_UID missing: run through pkexec"))
        .and_then(|s| parse_uid(&s))
    {
        Ok(u) => u,
        Err(e) => {
            eprintln!("akm-helper install-keymap: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    match apply(verb, &Paths::for_uid(uid), uid, &SystemRunner) {
        Ok(m) => {
            println!("{m}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("akm-helper install-keymap: {m}");
            ExitCode::from(1)
        }
    }
}
