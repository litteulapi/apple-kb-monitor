//! `apple-kb-monitord` — headless Apple keyboard monitor (systemd --user).
//!
//! ```text
//! apple-kb-monitord                      run the daemon (owns the keyboard)
//! apple-kb-monitord --json               print the state as JSON (D-Bus client;
//!                                        direct read if the daemon is absent)
//! apple-kb-monitord --json --daemon-only never touch the hardware (widget)
//! apple-kb-monitord --batteries [--json] battery sets and their lifetime
//! options: --no-bluez-provider --no-notify --no-history --threshold N
//!          --no-connection-notify --config PATH --bus-name NAME
//! ```
//!
//! Settings come from `$XDG_CONFIG_HOME/apple-kb-monitor/config.toml`
//! (see `akm_core::config`); `--threshold N` replaces the thresholds by `[N]`.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use akm_core::alerts::AlertConfig;
use akm_core::history::History;
use akm_core::{batteries, config, Watch};
use apple_kb_monitord::{actor, client, service, settings};

mod tray;

const USAGE: &str = "usage: apple-kb-monitord [--json [--daemon-only]] [--batteries [--json]] [--no-bluez-provider] [--no-notify] [--no-connection-notify] [--no-history] [--threshold N] [--config PATH] [--bus-name NAME] [--version]";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

#[derive(Debug, Default)]
struct Args {
    json: bool,
    daemon_only: bool,
    batteries: bool,
    opts: actor::Options,
    threshold: Option<f64>,
    no_connection_notify: bool,
    config: Option<std::path::PathBuf>,
    bus_name: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut a = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => a.json = true,
            "--daemon-only" => a.daemon_only = true,
            "--no-bluez-provider" => a.opts.bluez_provider = false,
            "--no-notify" => a.opts.notify = false,
            "--no-history" => a.opts.history = false,
            "--no-connection-notify" => a.no_connection_notify = true,
            "--batteries" => a.batteries = true,
            "--threshold" => {
                let v = it.next().ok_or("--threshold needs a value")?;
                a.threshold = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|t| (1.0..=95.0).contains(t))
                        .ok_or(format!("invalid threshold: {v}"))?,
                );
            }
            "--config" => {
                a.config = Some(it.next().ok_or("--config needs a path")?.into());
            }
            "--bus-name" => {
                let v = it.next().ok_or("--bus-name needs a name")?;
                zbus::names::WellKnownName::try_from(v.as_str())
                    .map_err(|e| format!("invalid bus name {v}: {e}"))?;
                a.bus_name = Some(v);
            }
            "--version" | "-V" => {
                println!("apple-kb-monitord {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown option: {other}\n{USAGE}")),
        }
    }
    if a.daemon_only && !a.json {
        return Err("--daemon-only only applies to --json".into());
    }
    if a.batteries && a.daemon_only {
        return Err("--daemon-only does not apply to --batteries".into());
    }
    Ok(Some(a))
}

/// Merge `config.toml` and the command line into the actor options.
fn effective_options(a: &Args) -> (actor::Options, Vec<String>) {
    let path = a.config.clone().unwrap_or_else(config::default_path);
    let (cfg, mut warnings) = config::load(&path);
    if a.config.is_some() && !path.exists() {
        warnings.push(format!("{}: not found; using defaults", path.display()));
    }
    let mut o = a.opts.clone();
    o.apply_config(&cfg);
    if let Some(t) = a.threshold {
        let c = &o.alerts;
        o.alerts = AlertConfig::new(vec![t.round() as u8], c.hysteresis, c.critical_at);
    }
    if a.no_connection_notify {
        o.notify_connection = false;
    }
    (o, warnings)
}

/// `--batteries`: battery sets from the history file (read-only).
fn print_batteries(json: bool) -> ExitCode {
    let sets = batteries::battery_sets(&History::open_default().read());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&sets).unwrap_or_default()
        );
    } else {
        print!("{}", batteries::format_sets(&sets));
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.batteries {
        return print_batteries(args.json);
    }
    if args.json {
        return match client::snapshot(!args.daemon_only) {
            Ok((s, src)) => {
                println!("{}", client::to_json(&s, src));
                if s.connected {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(3)
            }
        };
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .without_time() // journald stamps lines itself
        .init();
    let (opts, warnings) = effective_options(&args);
    for w in warnings {
        tracing::warn!("config: {w}");
    }
    run(opts, args.bus_name)
}

fn run(opts: actor::Options, bus_name: Option<String>) -> ExitCode {
    // SAFETY: the handler only stores into an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    let watch = Arc::new(Watch::new());
    let mailbox = actor::Mailbox::new();
    let history = opts.history.then(|| Arc::new(History::open_default()));

    // Take the bus name BEFORE touching the hardware: a second instance must
    // never open the keyboard.
    let mut so = service::ServeOptions::new(watch.clone(), mailbox.clone(), history);
    so.events = opts.events.clone();
    so.settings = Arc::new(settings::HelperBackend);
    if let Some(n) = bus_name {
        so.bus_name = n;
    }
    let conn = match service::serve_with(so) {
        Ok(c) => Some(c),
        Err(service::ServeError::NameTaken) => {
            tracing::error!("{}", service::ServeError::NameTaken);
            return ExitCode::from(1);
        }
        Err(e) => {
            // Headless without a session bus: keep BlueZ + history running.
            tracing::warn!("{e}; running without the session interface");
            None
        }
    };
    tracing::info!(
        "apple-kb-monitord {} started (bluez provider: {}, notifications: {}, history: {}, alerts: {:?}, connection notifications: {})",
        env!("CARGO_PKG_VERSION"),
        opts.bluez_provider,
        opts.notify,
        opts.history,
        opts.alerts_enabled.then(|| opts.alerts.thresholds().to_vec()),
        opts.notify_connection
    );
    tray::spawn(watch.clone(), mailbox.clone(), conn.clone());
    let handle = actor::spawn(watch, mailbox, opts);
    while !STOP.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(300));
    }
    tracing::info!("stopping");
    handle.stop();
    drop(conn);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Option<Args>, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments() {
        let a = p(&[]).unwrap().unwrap();
        assert!(!a.json && a.opts.bluez_provider && a.opts.notify && a.opts.history);
        let a = p(&["--json", "--daemon-only"]).unwrap().unwrap();
        assert!(a.json && a.daemon_only);
        let a = p(&[
            "--no-bluez-provider",
            "--no-notify",
            "--no-history",
            "--threshold",
            "20",
        ])
        .unwrap()
        .unwrap();
        assert!(!a.opts.bluez_provider && !a.opts.notify && !a.opts.history);
        assert_eq!(a.threshold, Some(20.0));
        let (o, _) = effective_options(&a);
        assert_eq!(o.alerts.thresholds(), &[20]);
        let a = p(&[
            "--no-connection-notify",
            "--config",
            "/nonexistent/c.toml",
            "--bus-name",
            "com.agenceapi.AppleKbMonitor1.Test",
        ])
        .unwrap()
        .unwrap();
        let (o, w) = effective_options(&a);
        assert!(!o.notify_connection);
        assert_eq!(o.alerts.thresholds(), &[30, 15, 5]);
        assert_eq!(w.len(), 1, "missing explicit config is reported");
        assert_eq!(
            a.bus_name.as_deref(),
            Some("com.agenceapi.AppleKbMonitor1.Test")
        );
        assert!(p(&["--bus-name", "not a name"]).is_err());
        assert!(p(&["--batteries", "--json"]).unwrap().unwrap().batteries);
        assert!(p(&["--batteries", "--daemon-only", "--json"]).is_err());
        assert!(p(&["--daemon-only"]).is_err());
        assert!(p(&["--threshold", "0"]).is_err());
        assert!(p(&["--threshold"]).is_err());
        assert!(p(&["--bogus"]).is_err());
    }
}
