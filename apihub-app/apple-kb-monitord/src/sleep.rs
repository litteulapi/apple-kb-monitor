//! System sleep through logind (#145).
//!
//! * A `delay` inhibitor is held while the system is awake. On
//!   `PrepareForSleep(true)` hardware access is paused ([`paused`]), the
//!   in-flight HID read (if any, see [`io_guard`]) is waited for at most
//!   [`IO_DRAIN`], then the inhibitor is released so the system can sleep.
//! * On `PrepareForSleep(false)` hardware access stays paused for
//!   [`RESUME_GRACE`] (the link is being rebuilt; a GET_REPORT now would only
//!   time out), the inhibitor is taken again and listeners are told.
//!
//! * Shutdown / restart (#191): a second `delay` inhibitor (`shutdown`) is held
//!   too. On `PrepareForShutdown(true)` the hook installed by
//!   [`set_shutdown_hook`] runs (it sends `WillShutdown`, bounded to a few
//!   seconds), then the inhibitor is released; on `PrepareForShutdown(false)`
//!   (shutdown cancelled) the inhibitor is taken again.
//!
//! The actor checks [`paused`] before every hardware action; the link keeper
//! ([`crate::repair`]) receives [`SleepEvent`]s.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator};
use zbus::zvariant::OwnedFd;
use zbus::MatchRule;

/// Longest wait for an in-flight hardware read before letting the system sleep.
pub const IO_DRAIN: Duration = Duration::from_secs(3);
/// Hardware stays paused this long after resume.
pub const RESUME_GRACE: Duration = Duration::from_secs(4);

static SLEEPING: AtomicBool = AtomicBool::new(false);
static IO_BUSY: AtomicUsize = AtomicUsize::new(0);
static RESUMED_AT: Mutex<Option<Instant>> = Mutex::new(None);

type Hook = Box<dyn Fn() + Send + Sync>;
static SHUTDOWN_HOOK: OnceLock<Hook> = OnceLock::new();

/// Register what runs on `PrepareForShutdown(true)` (once per process).
pub fn set_shutdown_hook(f: impl Fn() + Send + Sync + 'static) {
    let _ = SHUTDOWN_HOOK.set(Box::new(f));
}

/// What the link keeper is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepEvent {
    Sleeping,
    Resumed,
}

/// True while the system prepares to sleep, sleeps, or has just resumed.
pub fn paused() -> bool {
    paused_at(Instant::now())
}

fn paused_at(now: Instant) -> bool {
    if SLEEPING.load(Ordering::SeqCst) {
        return true;
    }
    let r = RESUMED_AT.lock().unwrap_or_else(|e| e.into_inner());
    r.is_some_and(|t| now.saturating_duration_since(t) < RESUME_GRACE)
}

/// Marks a hardware access in progress for the lifetime of the guard.
pub struct IoGuard(());

impl Drop for IoGuard {
    fn drop(&mut self) {
        IO_BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn io_guard() -> IoGuard {
    IO_BUSY.fetch_add(1, Ordering::SeqCst);
    IoGuard(())
}

fn set_sleeping() {
    SLEEPING.store(true, Ordering::SeqCst);
}

fn set_resumed(now: Instant) {
    *RESUMED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
    SLEEPING.store(false, Ordering::SeqCst);
}

/// Wait until no hardware access is in flight, at most `max`.
fn drain_io(max: Duration) -> bool {
    let end = Instant::now() + max;
    while IO_BUSY.load(Ordering::SeqCst) > 0 {
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

fn take_inhibitor(calls: &Connection) -> zbus::Result<OwnedFd> {
    take_inhibitor_for(calls, "sleep", "Pause keyboard reads before the Bluetooth link goes down")
}

fn take_inhibitor_for(calls: &Connection, what: &str, why: &str) -> zbus::Result<OwnedFd> {
    calls
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &(what, "apple-kb-monitord", why, "delay"),
        )?
        .body()
        .deserialize()
}

/// Spawn the logind watcher; `on_event` gets every transition. Without
/// logind (container, headless) the thread retries every minute, quietly.
pub fn spawn(on_event: impl Fn(SleepEvent) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("kb-sleep".into())
        .spawn(move || {
            let mut warned = false;
            loop {
                if let Err(e) = watch_once(&on_event) {
                    if !warned {
                        tracing::warn!(
                            "logind unavailable ({e}): sleep handling off, retry every 60 s"
                        );
                        warned = true;
                    }
                    // never leave the hardware paused because logind vanished
                    SLEEPING.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
        });
}

fn take_shutdown_inhibitor(calls: &Connection) -> Option<OwnedFd> {
    match take_inhibitor_for(calls, "shutdown", "Tell the keyboard the computer is shutting down (WillShutdown)") {
        Ok(fd) => Some(fd),
        Err(e) => {
            tracing::warn!("no shutdown inhibitor ({e}): WillShutdown may be cut short");
            None
        }
    }
}

fn watch_once(on_event: &dyn Fn(SleepEvent)) -> zbus::Result<()> {
    let conn = Connection::system()?;
    let calls = Connection::system()?;
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.login1")?
        .interface("org.freedesktop.login1.Manager")?
        .member("PrepareForSleep")?
        .build();
    let dbus = DBusProxy::new(&conn)?;
    dbus.add_match_rule(rule)?;
    dbus.add_match_rule(
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.login1")?
            .interface("org.freedesktop.login1.Manager")?
            .member("PrepareForShutdown")?
            .build(),
    )?;
    let mut shutdown_inhibitor = take_shutdown_inhibitor(&calls);
    let mut inhibitor = match take_inhibitor(&calls) {
        Ok(fd) => Some(fd),
        Err(e) => {
            tracing::warn!("no sleep inhibitor ({e}): reads may race the suspend");
            None
        }
    };
    tracing::info!(
        "sleep handling active (logind delay inhibitor: {})",
        inhibitor.is_some()
    );
    for msg in MessageIterator::from(conn) {
        let msg = msg?;
        let hdr = msg.header();
        let member = hdr.member().map(|m| m.as_str().to_owned());
        let Ok(start) = msg.body().deserialize::<bool>() else {
            continue;
        };
        if member.as_deref() == Some("PrepareForShutdown") {
            if start {
                // Shutdown or restart: tell the keyboard once, then let go.
                if let Some(h) = SHUTDOWN_HOOK.get() {
                    h();
                }
                shutdown_inhibitor = None;
            } else if shutdown_inhibitor.is_none() {
                shutdown_inhibitor = take_shutdown_inhibitor(&calls);
            }
            continue;
        }
        if member.as_deref() != Some("PrepareForSleep") {
            continue;
        }
        let now = Instant::now();
        if start {
            set_sleeping();
            on_event(SleepEvent::Sleeping);
            let drained = drain_io(IO_DRAIN);
            tracing::info!(
                "system going to sleep: keyboard access paused{}",
                if drained {
                    ""
                } else {
                    " (a read was still running)"
                }
            );
            inhibitor = None; // release: the system may sleep now
        } else {
            set_resumed(now);
            tracing::info!("system resumed: keyboard access resumes in {RESUME_GRACE:?}");
            on_event(SleepEvent::Resumed);
            if inhibitor.is_none() {
                inhibitor = take_inhibitor(&calls).ok();
            }
        }
    }
    drop(inhibitor);
    drop(shutdown_inhibitor);
    Err(zbus::Error::Failure("logind signal stream ended".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the state is process-global.
    #[test]
    fn pause_resume_grace_and_io_drain() {
        let t0 = Instant::now();
        assert!(!paused_at(t0));
        set_sleeping();
        assert!(paused_at(t0 + Duration::from_secs(3600)));
        set_resumed(t0 + Duration::from_secs(10));
        assert!(
            paused_at(t0 + Duration::from_secs(12)),
            "grace after resume"
        );
        assert!(!paused_at(t0 + Duration::from_secs(10) + RESUME_GRACE));
        // io drain
        let g = io_guard();
        assert!(!drain_io(Duration::from_millis(120)));
        drop(g);
        assert!(drain_io(Duration::from_millis(10)));
        *RESUMED_AT.lock().unwrap() = None;
    }
}
