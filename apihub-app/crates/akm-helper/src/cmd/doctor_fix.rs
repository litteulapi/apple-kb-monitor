//! `akm-helper doctor-fix` — applies the corrections `akmctl doctor` proposes.

use std::process::ExitCode;

use akm_helper::doctor_apply::{run as apply, Paths};
use akm_helper::doctor_fix::parse_args;
use akm_helper::keymap_install::SystemRunner;

use akm_helper::EX_USAGE;

pub fn run(argv: &[String]) -> ExitCode {
    let req = match parse_args(argv) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akm-helper doctor-fix: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let (lines, result) = apply(&req, &Paths::system(), &SystemRunner);
    for l in lines {
        println!("{l}");
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("akm-helper doctor-fix: {e}");
            ExitCode::from(1)
        }
    }
}
