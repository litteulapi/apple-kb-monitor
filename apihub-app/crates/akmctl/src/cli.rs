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

/// Clap value parser of the name stored IN the keyboard: exactly what would
/// be written (nothing trimmed), [`akm_core::devname::validate`].
fn parse_device_name(s: &str) -> Result<String, String> {
    akm_core::devname::validate(s).map_err(|e| e.to_string())
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
    /// into the keyboard). With --device-name: the name stored IN the keyboard
    #[command(
        after_help = "TWO NAMES:\n  akmctl rename <name>                 alias of THIS computer (BlueZ Alias): default, no risk\n  akmctl rename --device-name <name>   name stored IN the keyboard (0x51-0x55): dry run by default;\n                                       --write-device-name writes after pre-flight, backup and typed\n                                       confirmation (REFUSED while the sequence is not proven)\n  akmctl rename --device-name --show   name stored in the keyboard, from the daemon's cache\n  akmctl rename --device-name --restore <backup.json> [--write-device-name]"
    )]
    Rename {
        /// New alias (max 64 characters, no control characters)
        #[arg(value_parser = parse_name, required_unless_present_any = ["reset", "device_name"], conflicts_with_all = ["reset", "device_name"])]
        name: Option<String>,
        /// Restore the keyboard's own name
        #[arg(long)]
        reset: bool,
        /// Keyboard to rename (default: the one the daemon reports)
        #[arg(long, value_name = "MAC")]
        mac: Option<String>,
        /// Act on the name stored IN the keyboard (printable ASCII, 1-32
        /// characters); without a value: with --show or --restore
        #[arg(long, value_name = "NAME", num_args = 0..=1, value_parser = parse_device_name, conflicts_with = "reset")]
        device_name: Option<Option<String>>,
        /// Show the bytes that would be sent, write nothing (the default)
        #[arg(long, requires = "device_name", conflicts_with_all = ["write_device_name", "name", "reset"])]
        dry_run: bool,
        /// Really write (interactive, after pre-flight, backup and the name typed again)
        #[arg(long, requires = "device_name", conflicts_with_all = ["name", "reset"])]
        write_device_name: bool,
        /// Show the name stored in the keyboard (daemon cache, no hardware read)
        #[arg(long, requires = "device_name", conflicts_with_all = ["restore", "dry_run", "write_device_name", "name", "reset"])]
        show: bool,
        /// Rewrite a backup made by a previous --device-name run
        #[arg(long, value_name = "BACKUP", requires = "device_name", conflicts_with_all = ["name", "reset"])]
        restore: Option<std::path::PathBuf>,
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
    /// Ask the daemon to send WillShutdown (Feature 0x40, id only) to the
    /// keyboard, as macOS does at shutdown: once, only if connected and
    /// `[apple] will_shutdown` is on. Run by the shutdown unit
    ShutdownNotify {
        /// Do nothing unless the system is shutting down or restarting
        /// (`systemctl is-system-running` = stopping): used by the unit, so
        /// that a logout or a service restart never sends it
        #[arg(long)]
        only_if_stopping: bool,
    },
    /// Send HID_CONTROL SUSPEND (0x13) or EXIT_SUSPEND (0x14) to the connected
    /// Apple keyboards, as macOS bluetoothd does at sleep and wake (#244): one
    /// byte on the HID control channel of bluetoothd, administrator
    /// authentication. The system units send it by themselves at every
    /// sleep/wake; this is the manual test. --dry-run writes nothing
    HidControl {
        #[arg(value_enum)]
        op: crate::hid_control::HidControlOp,
        /// Only this keyboard (must be a connected Apple keyboard)
        #[arg(long, value_name = "MAC")]
        mac: Option<String>,
        /// Show the bluetoothd socket that would be used (pid, fd, MAC, PSM,
        /// state) and send nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Register map of the keyboard (every known HID report: name, size,
    /// safety class) with the values the daemon has cached, "never read"
    /// otherwise. Never reads the keyboard
    Info {
        /// Machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Firmware version of the keyboard against the embedded table of the
    /// latest public versions (read once per connection by the daemon; no
    /// network, never flashes)
    Firmware {
        /// Machine-readable JSON
        #[arg(long)]
        json: bool,
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
    /// Effective table of the special keys (F1-F12, Eject; --all: every known
    /// key): physical key -> evdev code (after hwdb and hid_apple, per fnmode)
    /// -> keysym -> KDE global shortcut. Never reads a key press
    Keys {
        /// Verdict per key ([ok] = a KDE shortcut is bound) and what to test
        #[arg(long)]
        check: bool,
        /// All known keys (modifiers, arrows, Fn...), not only the top row
        #[arg(long)]
        all: bool,
        /// Machine-readable JSON (with the KDE actions)
        #[arg(long)]
        json: bool,
    },
    /// Manual key mapping without keyd ($XDG_CONFIG_HOME/apple-kb-monitor/keymap.toml
    /// -> udev hwdb, installed with administrator authentication)
    Keymap {
        #[command(subcommand)]
        cmd: KeymapCmd,
    },
    /// Health self-check (daemon, link, journal, crashes, window, disk), run by apple-kb-monitor-selfcheck.timer
    Selftest(crate::selftest::SelftestArgs),
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
    /// hid_apple parameters (all of them, or one)
    Param {
        #[arg(value_parser = crate::keymapcmd::parse_param_name)]
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum KeymapCmd {
    /// Show the profiles of keymap.toml (--hwdb: the file `apply` would install)
    Show {
        #[arg(long)]
        hwdb: bool,
        #[arg(long)]
        json: bool,
    },
    /// Remap a key: KEY = F1..F12, Eject, Fn, LeftCmd... or a HID usage (0x7003a);
    /// CODE = a KEY_* name of linux/input-event-codes.h. Applied by `apply`
    Set {
        #[arg(value_parser = crate::keymapcmd::parse_key)]
        key: u32,
        #[arg(value_parser = crate::keymapcmd::parse_code)]
        code: u16,
        /// Profile to edit (default: the active one)
        #[arg(long, value_parser = crate::keymapcmd::parse_profile)]
        profile: Option<String>,
    },
    /// Back to the kernel mapping for one key (in keymap.toml)
    Unset {
        #[arg(value_parser = crate::keymapcmd::parse_key)]
        key: u32,
        #[arg(long, value_parser = crate::keymapcmd::parse_profile)]
        profile: Option<String>,
    },
    /// Preset of a profile (hid_apple parameters): apple (Apple legend,
    /// fnmode=1 swap_opt_cmd=0), fkeys (F1-F12 first, fnmode=2), linux-pc
    /// (Cmd<->Alt, swap_opt_cmd=1), none (parameters left as they are, the default)
    Preset {
        #[arg(value_parser = crate::keymapcmd::parse_preset)]
        name: String,
        #[arg(long, value_parser = crate::keymapcmd::parse_profile)]
        profile: Option<String>,
    },
    /// Select the active profile (created with the apple preset if new)
    Use {
        #[arg(value_parser = crate::keymapcmd::parse_profile)]
        profile: String,
    },
    /// Install the active profile: udev hwdb (akm-keymap-helper) and hid_apple
    /// parameters (akm-helper, persistent, ALL Apple keyboards)
    Apply {
        /// Show what would change, change nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the installed hwdb (kernel mapping back at once) and clear the
    /// keys of the active profile
    Reset,
    /// Restore the previously installed hwdb file
    Rollback,
    /// Add the KDE shortcuts the Apple legend needs and KDE lacks (F4 =
    /// Launch (D) -> application launcher), never replacing a binding
    KdeApply {
        #[arg(long)]
        dry_run: bool,
        /// Remove exactly the keys kde-apply added
        #[arg(long, conflicts_with = "dry_run")]
        undo: bool,
    },
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
    /// Set a hid_apple parameter (fnmode 0-4, iso_layout -1..1, swap_opt_cmd 0-2,
    /// swap_ctrl_cmd 0-1, swap_fn_leftctrl 0-1). Global to ALL Apple keyboards
    Param {
        #[arg(value_parser = crate::keymapcmd::parse_param_name)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Also write it to /etc/modprobe.d/hid_apple.conf
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
            Command::Rename {
                name,
                reset,
                mac,
                device_name,
                ..
            } => {
                assert_eq!(
                    (name.as_deref(), reset, mac, device_name),
                    (Some("Bureau"), false, None, None)
                );
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
    fn rename_device_name_arguments() {
        let c = |a: &[&str]| Cli::try_parse_from([&["akmctl", "rename"], a].concat());
        match c(&["--device-name", "Clavier de maria #1"])
            .unwrap()
            .command
        {
            Command::Rename {
                name,
                device_name,
                dry_run,
                write_device_name,
                show,
                restore,
                ..
            } => {
                assert_eq!(name, None);
                assert_eq!(device_name, Some(Some("Clavier de maria #1".into())));
                assert!(!dry_run && !write_device_name && !show && restore.is_none());
            }
            _ => panic!(),
        }
        assert!(c(&["--device-name", "x", "--dry-run"]).is_ok());
        assert!(c(&["--device-name", "x", "--write-device-name"]).is_ok());
        assert!(c(&["--device-name", "x", "--dry-run", "--write-device-name"]).is_err());
        assert!(matches!(
            c(&["--device-name", "--show"]).unwrap().command,
            Command::Rename {
                device_name: Some(None),
                show: true,
                ..
            }
        ));
        assert!(matches!(
            c(&["--device-name", "--restore", "/x.json"])
                .unwrap()
                .command,
            Command::Rename {
                device_name: Some(None),
                restore: Some(_),
                ..
            }
        ));
        // Never without --device-name; never mixed with the alias.
        for bad in [
            &["--write-device-name", "x"][..],
            &["--show"][..],
            &["--dry-run", "x"][..],
            &["--restore", "/x"][..],
            &["Bureau", "--device-name", "x"][..],
            &["--reset", "--device-name", "x"][..],
            &["--device-name", "--show", "--write-device-name"][..],
            &["--device-name", " x"][..],
            &["--device-name", "caf\u{e9}"][..],
            &["--device-name", "a\\b"][..],
            &["--reset", "--write-device-name"][..],
            &["--reset", "--dry-run"][..],
            &["--reset", "--show"][..],
            &["--reset", "--restore", "/x"][..],
            &["x", "--show"][..],
        ] {
            assert!(c(bad).is_err(), "{bad:?}");
        }
        assert!(c(&["--device-name", &"x".repeat(33)]).is_err());
        assert!(c(&["--device-name", &"x".repeat(32)]).is_ok());
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
        assert!(Cli::try_parse_from(["akmctl", "get", "param"]).is_ok());
        assert!(Cli::try_parse_from(["akmctl", "get", "param", "swap_opt_cmd"]).is_ok());
        assert!(Cli::try_parse_from(["akmctl", "get", "param", "rightalt_as_rightctrl"]).is_err());
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

    #[test]
    fn keys_and_keymap_parse() {
        let p = |a: &[&str]| Cli::try_parse_from([&["akmctl"], a].concat());
        assert!(matches!(p(&["keys", "--check"]).unwrap().command, Command::Keys { check: true, all: false, json: false }));
        match p(&["keymap", "set", "f6", "KEY_F13"]).unwrap().command {
            Command::Keymap { cmd: KeymapCmd::Set { key, code, profile } } => assert_eq!((key, code, profile), (0x7003f, 183, None)),
            c => panic!("{c:?}"),
        }
        assert!(p(&["keymap", "set", "0xc00b8", "KEY_DELETE", "--profile", "work"]).is_ok());
        for bad in [
            &["keymap", "set", "F13", "KEY_F1"][..],
            &["keymap", "set", "F1", "f2"],
            &["keymap", "set", "F1", "KEY_F2\nKEY_F3"],
            &["keymap", "set", "0x10001", "KEY_A"],
            &["keymap", "set", "F1", "KEY_F2", "--profile", "../x"],
            &["keymap", "preset", "mac"],
            &["keymap", "use", "Work"],
            &["keymap", "kde-apply", "--dry-run", "--undo"],
            &["set", "param", "ejectcd_as_delete", "1"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
        assert!(p(&["keymap", "preset", "linux-pc"]).is_ok() && p(&["keymap", "preset", "none"]).is_ok() && p(&["keymap", "apply", "--dry-run"]).is_ok());
        assert!(p(&["keymap", "reset"]).is_ok() && p(&["keymap", "rollback"]).is_ok() && p(&["keymap", "show", "--hwdb"]).is_ok());
        assert!(p(&["set", "param", "iso_layout", "-1", "--persist"]).is_ok());
    }
}
