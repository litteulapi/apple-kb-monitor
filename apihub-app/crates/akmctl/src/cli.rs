//! Command-line definition.

use clap::{Parser, Subcommand, ValueEnum};

use crate::fnmode::parse_mode;

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
