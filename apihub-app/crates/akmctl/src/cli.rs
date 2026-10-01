//! Command-line definition.

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::fnmode::parse_mode;
use crate::ledcmd::{LedName, LedState};

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
    /// Battery history (single store: $XDG_STATE_HOME/apple-kb-monitor/history.jsonl)
    History(HistoryArgs),
    /// Terminal chart of the battery (percentage and voltage) over 24 h or 7 days
    Graph {
        /// Time window
        #[arg(long, value_enum, default_value = "24h")]
        span: Span,
    },
    /// JSON for a waybar/polybar custom module (classes: good, warning, critical, disconnected)
    Waybar,
    /// Prometheus text exposition of the keyboard state (one page)
    Metrics,
    /// Switch a keyboard LED (NumLock can only be switched off)
    Led {
        #[arg(value_enum, ignore_case = true)]
        name: LedName,
        #[arg(value_enum, ignore_case = true)]
        state: LedState,
    },
    /// Read the 3 allowed HID reports (0x47, 0x46, 0x49) with the safe read
    /// policy: no scan, never 0x4C / 0xFE / 0x01. Press a key first (an idle
    /// keyboard is left alone)
    Dump {
        /// Machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Diagnose the Bluetooth link in one command (BlueZ, pairing, adapter
    /// power management, BlueZ/UPower configuration, journal, hidraw, daemon).
    /// Read-only; run with sudo to also compare the stored and kernel link keys
    Doctor {
        /// Machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Keyboard to check (default: the paired one)
        #[arg(long, value_name = "MAC")]
        mac: Option<String>,
    },
    /// Guided repair of the link: wake + reconnect first; re-pair only after a
    /// typed confirmation (the pairing is never removed otherwise)
    Repair {
        /// Keyboard to repair (default: the paired one)
        #[arg(long, value_name = "MAC")]
        mac: Option<String>,
        /// Offer re-pairing even if the link looks healthy
        #[arg(long)]
        force: bool,
    },
    /// Print a shell completion script on stdout
    Completions { shell: Shell },
    /// Print the manual page (roff) on stdout
    Man,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    #[value(name = "24h")]
    Day,
    #[value(name = "7d")]
    Week,
}

impl Span {
    pub fn seconds(self) -> u64 {
        match self {
            Span::Day => 24 * 3600,
            Span::Week => 7 * 24 * 3600,
        }
    }
}

/// Date filters shared by `history` and `history export`.
#[derive(Args, Debug, Clone)]
pub struct Filter {
    /// Oldest entry: 90m, 24h, 7d, 2w, YYYY-MM-DD or "YYYY-MM-DD HH:MM" (UTC)
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
    /// Newest entry (same formats)
    #[arg(long, value_name = "WHEN")]
    pub until: Option<String>,
    /// Keep only the last N entries
    #[arg(long, value_name = "N")]
    pub last: Option<usize>,
}

#[derive(Args, Debug)]
pub struct HistoryArgs {
    #[command(subcommand)]
    pub cmd: Option<HistoryCmd>,
    #[command(flatten)]
    pub filter: Filter,
    /// JSON array on one line instead of the table
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum HistoryCmd {
    /// Export the history on stdout
    Export {
        /// CSV format (the only one; header included)
        #[arg(long, required = true)]
        csv: bool,
        #[command(flatten)]
        filter: Filter,
    },
    /// Import the former history files into the single store: the Python CLI
    /// one ($XDG_RUNTIME_DIR/apple-kb-monitor/history.jsonl by default). The
    /// source is not modified; existing lines are not duplicated
    Import {
        /// File to import (default: the Python CLI history)
        file: Option<std::path::PathBuf>,
    },
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

    #[test]
    fn history_graph_waybar_metrics_led_dump_parse() {
        let p = |a: &[&str]| Cli::try_parse_from([&["akmctl"], a].concat());
        assert!(matches!(p(&["history"]).unwrap().command, Command::History(HistoryArgs { cmd: None, json: false, .. })));
        match p(&["history", "--since", "7d", "--last", "5", "--json"]).unwrap().command {
            Command::History(h) => assert_eq!((h.filter.since.as_deref(), h.filter.last, h.json), (Some("7d"), Some(5), true)),
            _ => panic!(),
        }
        assert!(matches!(
            p(&["history", "export", "--csv", "--until", "2026-10-01"]).unwrap().command,
            Command::History(HistoryArgs { cmd: Some(HistoryCmd::Export { csv: true, .. }), .. })
        ));
        assert!(p(&["history", "export"]).is_err(), "--csv is required");
        assert!(p(&["history", "import"]).is_ok());
        assert!(matches!(p(&["graph"]).unwrap().command, Command::Graph { span: Span::Day }));
        assert!(matches!(p(&["graph", "--span", "7d"]).unwrap().command, Command::Graph { span: Span::Week }));
        assert!(p(&["graph", "--span", "3d"]).is_err());
        assert!(p(&["waybar"]).is_ok() && p(&["metrics"]).is_ok());
        assert!(matches!(p(&["dump", "--json"]).unwrap().command, Command::Dump { json: true }));
        assert!(matches!(p(&["led", "caps", "on"]).unwrap().command, Command::Led { name: LedName::Caps, state: LedState::On }));
        assert!(p(&["led", "CapsLock", "OFF"]).is_ok());
        assert!(p(&["led", "caps"]).is_err() && p(&["led", "caps", "dim"]).is_err() && p(&["led", "shift", "on"]).is_err());
        assert_eq!(Span::Week.seconds(), 604_800);
    }
}
