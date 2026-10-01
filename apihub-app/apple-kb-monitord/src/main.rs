//! `apple-kb-monitord` — headless Apple keyboard monitor (systemd --user).
//!
//! ```text
//! apple-kb-monitord                      run the daemon (owns the keyboard)
//! apple-kb-monitord --json               print the state as JSON (D-Bus client;
//!                                        direct read if the daemon is absent)
//! apple-kb-monitord --json --daemon-only never touch the hardware (widget)
//! options: --no-bluez-provider --no-notify --no-history --threshold N
//! ```

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use akm_core::history::History;
use akm_core::Watch;
use apple_kb_monitord::{actor, client, service};

mod tray;

const USAGE: &str = "usage: apple-kb-monitord [--json [--daemon-only]] [--no-bluez-provider] [--no-notify] [--no-history] [--threshold N] [--version]";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

#[derive(Debug, Default)]
struct Args {
    json: bool,
    daemon_only: bool,
    opts: actor::Options,
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
            "--threshold" => {
                let v = it.next().ok_or("--threshold needs a value")?;
                a.opts.low_threshold = v
                    .parse::<f64>()
                    .ok()
                    .filter(|t| (1.0..=95.0).contains(t))
                    .ok_or(format!("invalid threshold: {v}"))?;
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
    Ok(Some(a))
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
    run(args.opts)
}

fn run(opts: actor::Options) -> ExitCode {
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
    let conn = match service::serve(watch.clone(), mailbox.clone(), history) {
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
        "apple-kb-monitord {} started (bluez provider: {}, notifications: {}, history: {})",
        env!("CARGO_PKG_VERSION"),
        opts.bluez_provider,
        opts.notify,
        opts.history
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
        assert_eq!(a.opts.low_threshold, 20.0);
        assert!(p(&["--daemon-only"]).is_err());
        assert!(p(&["--threshold", "0"]).is_err());
        assert!(p(&["--threshold"]).is_err());
        assert!(p(&["--bogus"]).is_err());
    }
}
