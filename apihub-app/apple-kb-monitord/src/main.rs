//! `apple-kb-monitord` — headless Apple keyboard monitor (systemd --user).

#[macro_use]
extern crate akm_core;

use std::process::ExitCode;
use std::sync::Arc;

use akm_core::alerts::AlertConfig;
use akm_core::history::History;
use akm_core::{batteries, config, Watch};
use apple_kb_monitord::{actor, client, service, settings};

mod tray;

const USAGE: &str = "usage: apple-kb-monitord [--json [--daemon-only]] [--batteries [--json]] [--no-bluez-provider] [--no-notify] [--no-connection-notify] [--no-history] [--threshold N] [--config PATH] [--bus-name NAME] [-h|--help] [-V|--version]";

/// Block SIGTERM/SIGINT in this thread and every thread it spawns later; `sigwait` takes them.
fn block_stop_signals() -> libc::sigset_t {
    // SAFETY: a zeroed sigset_t is valid storage for sigemptyset; all pointers are valid.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut set);
        libc::sigaddset(&raw mut set, libc::SIGTERM);
        libc::sigaddset(&raw mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
        set
    }
}

fn wait_stop_signal(set: &libc::sigset_t) -> libc::c_int {
    let mut sig = 0;
    // SAFETY: `set` is initialised and `sig` is a valid out pointer.
    while unsafe { libc::sigwait(set, &raw mut sig) } != 0 {}
    sig
}

#[allow(clippy::struct_excessive_bools)] // independent command-line flags
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

fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Option<Args>, String> {
    let mut a = Args::default();
    let mut it = args.into_iter().map(|s| {
        s.into_string()
            .map_err(|s| format!("invalid argument (not UTF-8): {}", s.to_string_lossy()))
    });
    while let Some(arg) = it.next() {
        let arg = arg?;
        match arg.as_str() {
            "--json" => a.json = true,
            "--daemon-only" => a.daemon_only = true,
            "--no-bluez-provider" => a.opts.bluez_provider = false,
            "--no-notify" => a.opts.notify = false,
            "--no-history" => a.opts.history = false,
            "--no-connection-notify" => a.no_connection_notify = true,
            "--batteries" => a.batteries = true,
            "--threshold" => {
                let v = it.next().ok_or("--threshold needs a value")??;
                a.threshold = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|t| (1.0..=95.0).contains(t))
                        .ok_or(format!("invalid threshold: {v}"))?,
                );
            }
            "--config" => {
                a.config = Some(it.next().ok_or("--config needs a path")??.into());
            }
            "--bus-name" => {
                let v = it.next().ok_or("--bus-name needs a name")??;
                zbus::names::WellKnownName::try_from(v.as_str())
                    .map_err(|e| format!("invalid bus name {v}: {e}"))?;
                a.bus_name = Some(v);
            }
            "--version" | "-V" => {
                println!("apple-kb-monitord {}", akm_core::PKG_VERSION);
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
        o.alerts = AlertConfig::new(vec![akm_core::conv::pct_u8(t)], c.hysteresis, c.critical_at);
    }
    if a.no_connection_notify {
        o.notify_connection = false;
    }
    (o, warnings)
}

fn print_batteries(json: bool) -> ExitCode {
    let groups = batteries::sets_by_keyboard(&History::open_default().read());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&groups).unwrap_or_default()
        );
    } else {
        print!("{}", batteries::format_by_keyboard(&groups));
    }
    ExitCode::SUCCESS
}

/// `EX_USAGE` of sysexits.h, as akmctl.
const EXIT_USAGE: u8 = 64;

fn main() -> ExitCode {
    akm_core::i18n::init();
    // The 1 s spacing between two requests to the keyboard holds across processes (daemon and
    // akmctl): share the instant of the last access.
    akm_core::read_policy::share_hw_access(true);
    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_USAGE);
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

/// The session interface (`Monitor`, devices, settings); `None` headless, `Err` when another
/// monitor owns the name.
fn serve_session(
    so: &service::ServeOptions,
    export: impl FnOnce(&zbus::blocking::Connection),
) -> Result<Option<zbus::blocking::Connection>, ExitCode> {
    // Every object is exported before the name is taken (D-Bus activation delivers at once).
    let conn = match service::serve_exporting(so, |conn| {
        let s = apple_kb_monitord::config_api::Settings::default();
        if let Err(e) = conn
            .object_server()
            .at(apple_kb_monitord::config_api::PATH, s)
        {
            tracing::warn!(
                "settings: cannot export {}: {e}",
                apple_kb_monitord::config_api::PATH
            );
        }
        export(conn);
    }) {
        Ok(c) => c,
        Err(service::ServeError::NameTaken) => {
            tracing::error!("{}", service::ServeError::NameTaken);
            return Err(ExitCode::from(1));
        }
        Err(e) => {
            // Headless without a session bus: keep BlueZ + history running.
            tracing::warn!("{e}; running without the session interface");
            return Ok(None);
        }
    };
    Ok(Some(conn))
}

fn run(mut opts: actor::Options, bus_name: Option<String>) -> ExitCode {
    // Before any thread exists, so that all of them inherit the mask.
    let stop_signals = block_stop_signals();
    let watch = Arc::new(Watch::new());
    let mailbox = actor::Mailbox::new();
    let history = opts.history.then(|| Arc::new(History::open_default()));

    // Take the bus name BEFORE touching the hardware.
    let mut so = service::ServeOptions::new(watch.clone(), mailbox.clone(), history);
    so.events = opts.events.clone();
    so.settings = Arc::new(settings::HelperBackend::default());
    // Fn mode remembered per keyboard, put back (or offered) when it reconnects.
    let reapply = apple_kb_monitord::reapply::Reapplier::new(
        opts.history
            .then(akm_core::device_settings::DeviceSettings::default_path),
        so.settings.clone(),
        opts.reapply_policy,
    );
    apple_kb_monitord::reapply::install(reapply.clone());
    so.reapply = Some(reapply.clone());
    opts.reapply = Some(reapply);
    so.alias = opts.alias.clone();
    if let Some(n) = bus_name {
        so.bus_name = n;
    }
    // Link quality: one record shared by the actor and the link keeper.
    let link_stats = apple_kb_monitord::linkq::Store::new(
        opts.history
            .then(akm_core::linkstats::LinkStats::default_path),
    );
    // Every paired keyboard as BlueZ sees it: the roster of the actor.
    let roster = apple_kb_monitord::repair::SharedStatus::default();
    // Link keeper: reconnection, system sleep, health, reconciliation (started after the actor).
    let link = apple_kb_monitord::repair::prepare(Some(link_stats.clone()), Some(roster.clone()));
    let conn = match serve_session(&so, |c| {
        tray::spawn(watch.clone(), mailbox.clone(), Some(c.clone()));
        if let Err(e) = apple_kb_monitord::repair::export(c, link.handle()) {
            tracing::warn!("link object not exported: {e}");
        }
    }) {
        Ok(c) => c,
        Err(code) => return code,
    };
    tracing::info!(
        "apple-kb-monitord {} started (bluez provider: {}, notifications: {}, history: {}, alerts: {:?}, connection notifications: {})",
        akm_core::PKG_VERSION,
        opts.bluez_provider,
        opts.notify,
        opts.history,
        opts.alerts_enabled.then(|| opts.alerts.thresholds().to_vec()),
        opts.notify_connection
    );
    // Quiet hours and what waits to be shown again.
    apple_kb_monitord::notify_policy::configure(
        opts.quiet_hours.clone(),
        opts.history
            .then(akm_core::deferred::DeferredStore::default_path),
    );
    // Usage statistics, off unless `[usage] active_time = true`.
    if opts.usage_active_time {
        let tracker = apple_kb_monitord::usage::Tracker::new(
            opts.history.then(akm_core::usage::UsageStats::default_path),
        );
        apple_kb_monitord::usage::subscribe(&tracker);
        tracing::info!("usage statistics on: active minutes per day (no key is recorded)");
        opts.usage = Some(tracker);
    }
    let usage = opts.usage.clone();
    // Plasma OSD at a change of Fn mode and at a Caps Lock press.
    if conn.is_some() {
        apple_kb_monitord::osd::spawn(
            apple_kb_monitord::osd::Osd::new(
                Arc::new(apple_kb_monitord::osd::PlasmaOsd::default()),
                opts.osd,
            ),
            watch.clone(),
        );
    }
    // What macOS tells the keyboard at shutdown (Feature 0x40, once).
    apple_kb_monitord::shutdown::install(opts.will_shutdown, watch.clone());
    // Passive listening to the keyboard's own input reports (never a request).
    apple_kb_monitord::passive::set_keyboard_alerts(opts.notify && opts.alerts_enabled);
    let _passive = conn.as_ref().and_then(|c| {
        apple_kb_monitord::passive::start_default(c.clone(), watch.clone())
            .map_err(|e| tracing::warn!("passive listener not started: {e}"))
            .ok()
    });
    let (link_mailbox, link_notify) = (mailbox.clone(), opts.notify);
    opts.link_stats = Some(link_stats);
    let link_alert = opts.notify_link_unstable;
    opts.roster = Some(roster);
    let handle = actor::spawn(watch, mailbox, opts);
    link.start(link_mailbox, link_notify, link_alert);
    wait_stop_signal(&stop_signals);
    tracing::info!("stopping");
    handle.stop();
    if let Some(u) = usage {
        u.flush();
    }
    drop(conn);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_lists_the_short_and_long_help_and_version_flags() {
        for flag in ["-h", "--help", "-V", "--version"] {
            assert!(
                USAGE.contains(&format!("{flag}]")) || USAGE.contains(&format!("{flag}|")),
                "{flag}"
            );
            assert!(matches!(parse_args([flag.into()]), Ok(None)), "{flag}");
        }
    }

    #[test]
    fn stop_signal_is_taken_by_sigwait() {
        let set = block_stop_signals();
        // SAFETY: SIGTERM is blocked in this thread, so it stays pending until sigwait.
        unsafe { libc::raise(libc::SIGTERM) };
        assert_eq!(wait_stop_signal(&set), libc::SIGTERM);
    }

    fn p(v: &[&str]) -> Result<Option<Args>, String> {
        parse_args(v.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn a_non_utf8_argument_is_a_usage_error() {
        use std::os::unix::ffi::OsStringExt;
        let bad = std::ffi::OsString::from_vec(vec![b'-', 0xff]);
        let e = parse_args([bad.clone()]).err().unwrap();
        assert!(e.contains("not UTF-8"), "{e}");
        assert!(parse_args(["--config".into(), bad]).is_err());
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
