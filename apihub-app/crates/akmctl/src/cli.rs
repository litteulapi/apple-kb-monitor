//! Command-line definition.

use clap::{Parser, Subcommand, ValueEnum};

use crate::fnmode::parse_mode;

/// Clap value parser of a keyboard name: validated, trimmed, never empty
/// (use `--reset` to restore the original name).
fn parse_name(s: &str) -> Result<String, String> {
    match akm_core::alias::validate(s) {
        Ok(n) if n.is_empty() => Err("empty name (use --reset to restore the keyboard's own name)".into()),
        Ok(n) => Ok(n),
        Err(e) => Err(e.to_string()),
    }
}

/// Exit codes: 0 OK, 1 error, 2 daemon absent, 64 usage error.
pub const EXIT_OK: u8 = 0;
pub const EXIT_ERROR: u8 = 1;
pub const EXIT_ABSENT: u8 = 2;
pub const EXIT_USAGE: u8 = 64;

#[derive(Parser, Debug)]
#[command(
    name = "akmctl",
    version,
    about = "Control and query the Apple keyboard monitor",
    long_about = "Control and query the Apple keyboard monitor (apple-kb-monitord).\n\n\
The Fn mode is a parameter of the hid_apple kernel module: it applies to ALL \
Apple keyboards connected to this computer, not to a single one. Changing it \
requires administrator rights (polkit authentication).",
    after_help = "EXIT CODES:\n  0   success\n  1   error (D-Bus failure, write refused, bad data)\n  2   daemon absent (apple-kb-monitord not on the session bus)\n  64  usage error (invalid arguments)\n\n\
FN MODES (hid_apple.fnmode, global to every Apple keyboard):\n  0  disabled   Fn key has no effect\n  1  fkeyslast  media keys by default, Fn+Fx = F-key (kernel default)\n  2  fkeysfirst F-keys by default, Fn+Fx = media key\n  3  auto       F-keys by default, media keys on Apple-layout apps",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show keyboard state (battery, connection, RSSI, Fn mode)
    Status {
        /// Machine-readable JSON (schema 1) on a single line
        #[arg(long)]
        json: bool,
    },
    /// Read a setting
    Get {
        #[command(subcommand)]
        what: GetCmd,
    },
    /// Change a setting (administrator authentication required)
    Set {
        #[command(subcommand)]
        what: SetCmd,
    },
    /// Rename the keyboard on this computer (BlueZ alias; nothing is written
    /// into the keyboard)
    Rename {
        /// New name (max 64 characters, no control characters)
        #[arg(value_parser = parse_name, required_unless_present = "reset", conflicts_with = "reset")]
        name: Option<String>,
        /// Restore the keyboard's own name
        #[arg(long)]
        reset: bool,
        /// Keyboard to rename (default: the one the daemon reports)
        #[arg(long, value_name = "MAC")]
        mac: Option<String>,
    },
    /// Follow StateChanged signals: one JSON line per change, until interrupted
    Watch,
    /// Print a shell completion script on stdout
    Completions { shell: Shell },
    /// Print the manual page (roff) on stdout
    Man,
}

#[derive(Subcommand, Debug)]
pub enum GetCmd {
    /// Current Fn mode (0-3), read from /sys/module/hid_apple/parameters/fnmode
    Fnmode,
}

#[derive(Subcommand, Debug)]
pub enum SetCmd {
    /// Set the Fn mode of hid_apple (0-3). Global to ALL Apple keyboards; asks for
    /// administrator authentication through polkit (pkexec akm-helper)
    Fnmode {
        /// 0 disabled, 1 fkeyslast, 2 fkeysfirst, 3 auto
        #[arg(value_parser = parse_mode)]
        mode: u8,
        /// Also write it to /etc/modprobe.d/hid_apple.conf so it survives reboots
        #[arg(long)]
        persist: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    #[value(name = "powershell")]
    Pwsh,
}

impl From<Shell> for clap_complete::Shell {
    fn from(s: Shell) -> Self {
        match s {
            Shell::Bash => Self::Bash,
            Shell::Zsh => Self::Zsh,
            Shell::Fish => Self::Fish,
            Shell::Elvish => Self::Elvish,
            Shell::Pwsh => Self::PowerShell,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn set_fnmode_validates_range() {
        for ok in ["0", "1", "2", "3"] {
            assert!(Cli::try_parse_from(["akmctl", "set", "fnmode", ok]).is_ok());
        }
        for bad in ["4", "-1", "x", "10", ""] {
            assert!(Cli::try_parse_from(["akmctl", "set", "fnmode", bad]).is_err(), "{bad:?}");
        }
        assert!(Cli::try_parse_from(["akmctl", "set", "fnmode"]).is_err());
    }

    #[test]
    fn rename_arguments() {
        let c = |a: &[&str]| Cli::try_parse_from([&["akmctl", "rename"], a].concat());
        match c(&["  Bureau  "]).unwrap().command {
            Command::Rename { name, reset, mac } => {
                assert_eq!((name.as_deref(), reset, mac), (Some("Bureau"), false, None));
            }
            _ => panic!(),
        }
        assert!(matches!(
            c(&["--reset"]).unwrap().command,
            Command::Rename { name: None, reset: true, .. }
        ));
        assert!(c(&["--mac", "04:DB:56:CA:42:EE", "x"]).is_ok());
        assert!(c(&[]).is_err(), "a name or --reset is required");
        assert!(c(&["x", "--reset"]).is_err());
        assert!(c(&["   "]).is_err());
        assert!(c(&["a\nb"]).is_err());
        assert!(c(&[&"x".repeat(65)]).is_err());
    }

    #[test]
    fn parses_other_commands() {
        assert!(matches!(
            Cli::try_parse_from(["akmctl", "status", "--json"]).unwrap().command,
            Command::Status { json: true }
        ));
        assert!(Cli::try_parse_from(["akmctl", "get", "fnmode"]).is_ok());
        assert!(Cli::try_parse_from(["akmctl", "watch"]).is_ok());
        assert!(Cli::try_parse_from(["akmctl", "get", "nothing"]).is_err());
        assert!(Cli::try_parse_from(["akmctl"]).is_err());
    }
}
