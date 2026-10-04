//! System sleep through logind.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator};
use zbus::zvariant::OwnedFd;
use zbus::MatchRule;

use crate::origin::Origin;

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
#[must_use]
pub fn paused() -> bool {
    paused_at(Instant::now())
}

fn paused_at(now: Instant) -> bool {
    if SLEEPING.load(Ordering::SeqCst) {
        return true;
    }
    let r = RESUMED_AT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    // The burst in progress stops before its next request: the one in flight fits in IO_DRAIN.
    akm_core::read_policy::hold(true);
}

fn set_resumed(now: Instant) {
    *RESUMED_AT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(now);
    SLEEPING.store(false, Ordering::SeqCst);
    akm_core::read_policy::hold(false);
}

/// Lift a pause left by a lost logind stream; the keeper hears `Resumed` once.
fn release_after_loss(now: Instant, on_event: &dyn Fn(SleepEvent)) {
    if SLEEPING.load(Ordering::SeqCst) {
        set_resumed(now);
        on_event(SleepEvent::Resumed);
    }
}

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
    take_inhibitor_for(calls, "sleep", &inhibitor_reason(Inhibit::Sleep))
}

/// Which delay lock a reason is worded for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inhibit {
    Sleep,
    Shutdown,
}

/// Text shown by `systemd-inhibit --list` and by session managers.
#[must_use]
pub fn inhibitor_reason(which: Inhibit) -> String {
    match which {
        Inhibit::Sleep => tr!("Pause keyboard reads before the Bluetooth link goes down"),
        Inhibit::Shutdown => tr!("Tell the keyboard the computer is shutting down (WillShutdown)"),
    }
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

/// Spawn the logind watcher; `on_event` gets every transition.
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
                    release_after_loss(Instant::now(), &on_event);
                    std::thread::sleep(Duration::from_mins(1));
                }
            }
        });
}

fn take_shutdown_inhibitor(calls: &Connection) -> Option<OwnedFd> {
    match take_inhibitor_for(calls, "shutdown", &inhibitor_reason(Inhibit::Shutdown)) {
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
    let mut origin = Origin::new(&calls, &["org.freedesktop.login1"]);
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
        if !origin.accept(&msg) {
            continue;
        }
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

    // SLEEPING and RESUMED_AT are process-wide.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn lost_stream_lifts_the_hold_once() {
        let _s = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let events = std::cell::RefCell::new(Vec::new());
        let t0 = Instant::now();
        set_sleeping();
        assert!(akm_core::read_policy::held());
        release_after_loss(t0, &|e| events.borrow_mut().push(e));
        assert!(!akm_core::read_policy::held(), "reads must not stay held");
        assert!(!paused_at(t0 + RESUME_GRACE));
        release_after_loss(t0, &|e| events.borrow_mut().push(e));
        assert_eq!(*events.borrow(), [SleepEvent::Resumed]);
        *RESUMED_AT.lock().unwrap() = None;
    }

    #[test]
    fn pause_resume_grace_and_io_drain() {
        let _s = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let t0 = Instant::now();
        assert!(!paused_at(t0));
        set_sleeping();
        assert!(paused_at(t0 + Duration::from_hours(1)));
        set_resumed(t0 + Duration::from_secs(10));
        assert!(
            paused_at(t0 + Duration::from_secs(12)),
            "grace after resume"
        );
        assert!(!paused_at(t0 + Duration::from_secs(10) + RESUME_GRACE));
        let g = io_guard();
        assert!(!drain_io(Duration::from_millis(120)));
        drop(g);
        assert!(drain_io(Duration::from_millis(10)));
        *RESUMED_AT.lock().unwrap() = None;
    }
}
