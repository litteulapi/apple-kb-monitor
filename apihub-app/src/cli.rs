//! Command line of `apihub-app` (#271): handled before the single-instance
//! claim, so that `--help` never activates a running window.

/// What one command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    /// No argument: open (or activate) the window.
    Window,
    ToggleFn,
    Krunner,
    Help,
    Version,
    /// Unknown argument: refused, nothing else happens.
    Bad(String),
}

pub fn parse(args: &[String]) -> Cli {
    match args {
        [] => Cli::Window,
        [a] => match a.as_str() {
            crate::fn_toggle::FLAG => Cli::ToggleFn,
            crate::krunner::FLAG => Cli::Krunner,
            "-h" | "--help" => Cli::Help,
            "-V" | "--version" => Cli::Version,
            other => Cli::Bad(other.to_string()),
        },
        [a, ..] if a == "-h" || a == "--help" => Cli::Help,
        [_, b, ..] => Cli::Bad(b.to_string()),
    }
}

pub fn help() -> String {
    use crate::i18n::tr;
    let opts = [
        ("-h, --help", tr("show this help and exit")),
        ("-V, --version", tr("show the version and exit")),
        (
            crate::fn_toggle::FLAG,
            tr("toggle the Fn mode through the service, without a window"),
        ),
        (
            crate::krunner::FLAG,
            tr("KRunner runner (started by KRunner itself)"),
        ),
    ];
    let mut out = format!(
        "{}\n\n{}\n\n{}\n",
        tr("Usage: apihub-app [OPTION]"),
        tr("Opens the Apple keyboard monitor window; if it is already open, brings it to the front."),
        tr("Options:"),
    );
    for (o, d) in opts {
        out.push_str(&format!("  {o:<15} {d}\n"));
    }
    out
}

pub fn version() -> String {
    format!("apihub-app {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Cli {
        parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn help_and_version_never_open_the_window() {
        assert_eq!(p(&["--help"]), Cli::Help);
        assert_eq!(p(&["-h"]), Cli::Help);
        assert_eq!(p(&["--version"]), Cli::Version);
        assert_eq!(p(&["-V"]), Cli::Version);
    }

    #[test]
    fn unknown_arguments_are_refused() {
        assert_eq!(p(&["--frobnicate"]), Cli::Bad("--frobnicate".into()));
        assert_eq!(p(&["fichier.txt"]), Cli::Bad("fichier.txt".into()));
        assert_eq!(p(&["--toggle-fn", "x"]), Cli::Bad("x".into()));
    }

    #[test]
    fn known_modes() {
        assert_eq!(p(&[]), Cli::Window);
        assert_eq!(p(&["--toggle-fn"]), Cli::ToggleFn);
        assert_eq!(p(&["--krunner"]), Cli::Krunner);
    }

    #[test]
    fn help_lists_every_option() {
        let h = help();
        for o in ["--help", "--version", "--toggle-fn", "--krunner"] {
            assert!(h.contains(o), "{o} missing from --help");
        }
    }
}
