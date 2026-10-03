//! Actions of the window, run **off the UI thread** and bounded in time:
//! reconnect the keyboard (`Link.Reconnect`), read and toggle the mode of
//! the function keys (`Device.FnMode` / `Device.SetFnMode`). Thin D-Bus
//! clients of the daemon: nothing here touches BlueZ, sysfs or the keyboard.
//!
//! One job at a time per action, so repeated clicks never pile up threads;
//! a daemon that never answers leaves an error after the timeout instead of
//! a button stuck on "busy" (same contract as `tab_keys`, #230).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use apple_kb_monitord::repair::{LINK_INTERFACE, LINK_PATH};
use apple_kb_monitord::service::BUS_NAME;
use zbus::blocking::Connection;

use crate::i18n::{tr, trf};

/// Longest wait for a read or a request the daemon answers at once.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest wait when the daemon opens a polkit dialog.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(150);

/// `(ok, message)` of the last run, shown under the buttons.
pub type Outcome = Option<(bool, String)>;

/// A one-shot background job with its outcome.
#[derive(Clone, Default)]
pub struct Job {
    busy: Arc<AtomicBool>,
    outcome: Arc<Mutex<(Outcome, Option<std::time::Instant>)>>,
}

/// How long a result stays on screen (#275): a success is a short
/// acknowledgement, a failure stays longer, both go at the next action.
pub const SHOW_SUCCESS: Duration = Duration::from_secs(10);
pub const SHOW_FAILURE: Duration = Duration::from_secs(60);

/// The result still worth showing `age` after it arrived.
pub fn still_shown(o: &Outcome, age: Duration) -> bool {
    match o {
        Some((true, _)) => age < SHOW_SUCCESS,
        Some((false, _)) => age < SHOW_FAILURE,
        None => false,
    }
}

impl Job {
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    pub fn outcome(&self) -> Outcome {
        let g = self.outcome.lock().unwrap_or_else(|e| e.into_inner());
        let age = g.1.map_or(Duration::ZERO, |t| t.elapsed());
        still_shown(&g.0, age).then(|| g.0.clone()).flatten()
    }

    fn set(&self, o: Outcome) {
        *self.outcome.lock().unwrap_or_else(|e| e.into_inner()) =
            (o, Some(std::time::Instant::now()));
    }

    /// Run `work` on a worker thread, waited for at most `timeout`; `done`
    /// is called when the outcome is known. Ignored while a job is running.
    pub fn start(
        &self,
        timeout: Duration,
        done: impl Fn() + Send + 'static,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        self.set(None);
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("action".into())
            .spawn(move || {
                let (tx, rx) = mpsc::channel();
                let inner = std::thread::Builder::new()
                    .name("action-dbus".into())
                    .spawn(move || {
                        let _ = tx.send(work());
                    });
                let res = match inner {
                    Err(e) => Err(e.to_string()),
                    Ok(_) => rx.recv_timeout(timeout).unwrap_or_else(|_| {
                        Err(trf(
                            "daemon did not answer within {} s",
                            &[&timeout.as_secs()],
                        ))
                    }),
                };
                me.set(Some(match res {
                    Ok(m) => (true, m),
                    Err(e) => (false, e),
                }));
                me.busy.store(false, Ordering::Release);
                done();
            });
        if spawned.is_err() {
            self.busy.store(false, Ordering::Release);
        }
    }
}

fn session() -> Result<Connection, String> {
    Connection::session().map_err(|e| trf("no session bus: {}", &[&e]))
}

/// Ask the daemon to page the keyboard now (`Link.Reconnect`).
pub fn reconnect_on(conn: &Connection) -> Result<String, String> {
    let asked: bool = conn
        .call_method(
            Some(BUS_NAME),
            LINK_PATH,
            Some(LINK_INTERFACE),
            "Reconnect",
            &(),
        )
        .and_then(|m| m.body().deserialize())
        .map_err(|e| trf("daemon unreachable: {}", &[&e]))?;
    if asked {
        Ok(tr("Reconnection requested: press a key on the keyboard").into())
    } else {
        Err(tr("The daemon refused the reconnection request").into())
    }
}

pub fn reconnect() -> Result<String, String> {
    reconnect_on(&session()?)
}

/// Mode of the function keys as the daemon reads it, and its toggle.
#[derive(Clone, Default)]
pub struct FnMode {
    /// `None`: not read (daemon absent, no keyboard known).
    mode: Arc<Mutex<Option<i32>>>,
    reading: Arc<AtomicBool>,
    pub job: Job,
}

impl FnMode {
    pub fn mode(&self) -> Option<i32> {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store(&self, m: Option<i32>) {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = m;
    }

    /// Read the current mode in the background (no message on failure: the
    /// buttons then show the mode as unknown).
    pub fn refresh(&self, done: impl Fn() + Send + 'static) {
        self.refresh_with(done, || {
            let c = session()?;
            let path = crate::fn_toggle::keyboard_path(&c)?;
            crate::fn_toggle::current_mode(&c, &path)
        });
    }

    pub fn refresh_with(
        &self,
        done: impl Fn() + Send + 'static,
        read: impl FnOnce() -> Result<i32, String> + Send + 'static,
    ) {
        if self.reading.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("fnmode-read".into())
            .spawn(move || {
                let (tx, rx) = mpsc::channel();
                let inner = std::thread::Builder::new()
                    .name("fnmode-dbus".into())
                    .spawn(move || {
                        let _ = tx.send(read());
                    });
                if inner.is_ok() {
                    if let Ok(r) = rx.recv_timeout(CALL_TIMEOUT) {
                        me.store(r.ok());
                    }
                }
                me.reading.store(false, Ordering::Release);
                done();
            });
        if spawned.is_err() {
            self.reading.store(false, Ordering::Release);
        }
    }

    /// Toggle through the daemon (polkit dialog), then show the new mode.
    pub fn toggle(&self, done: impl Fn() + Send + 'static) {
        self.toggle_with(done, || {
            crate::fn_toggle::toggle(&session()?).map(|(_, new)| new)
        });
    }

    pub fn toggle_with(
        &self,
        done: impl Fn() + Send + 'static,
        toggle: impl FnOnce() -> Result<i32, String> + Send + 'static,
    ) {
        let me = self.clone();
        self.job.start(AUTH_TIMEOUT, done, move || {
            let new = toggle()?;
            me.store(Some(new));
            Ok(trf("Fn mode: {}", &[&crate::fn_toggle::mode_text(new)]))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::FakeDaemon;
    use std::time::Instant;

    /// #275: a result does not stay for minutes.
    #[test]
    fn results_expire() {
        let ok: Outcome = Some((true, "done".into()));
        let ko: Outcome = Some((false, "failed".into()));
        assert!(still_shown(&ok, Duration::from_secs(1)));
        assert!(!still_shown(&ok, SHOW_SUCCESS));
        assert!(still_shown(&ko, SHOW_SUCCESS));
        assert!(!still_shown(&ko, SHOW_FAILURE));
        assert!(!still_shown(&None, Duration::ZERO));
    }

    fn wait(job: &Job) -> Outcome {
        let t = Instant::now();
        while job.busy() {
            assert!(t.elapsed() < Duration::from_secs(10), "job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
        job.outcome()
    }

    #[test]
    fn a_job_reports_its_outcome_and_refuses_a_second_run() {
        let job = Job::default();
        assert_eq!(job.outcome(), None);
        let (tx, rx) = mpsc::channel::<()>();
        job.start(
            CALL_TIMEOUT,
            || {},
            move || {
                let _ = rx.recv();
                Ok("done".into())
            },
        );
        assert!(job.busy());
        // Second click while running: ignored, the work is never called.
        job.start(CALL_TIMEOUT, || {}, || unreachable!("second job"));
        tx.send(()).unwrap();
        assert_eq!(wait(&job), Some((true, "done".into())));
        job.start(CALL_TIMEOUT, || {}, || Err("nope".into()));
        assert_eq!(wait(&job), Some((false, "nope".into())));
    }

    /// A daemon that never answers: the UI-side call returns at once and
    /// the job ends with an error at the timeout (#230).
    #[test]
    fn a_stuck_job_ends_at_the_timeout() {
        let job = Job::default();
        let t = Instant::now();
        job.start(
            Duration::from_millis(200),
            || {},
            || {
                std::thread::sleep(Duration::from_secs(3600));
                Ok(String::new())
            },
        );
        assert!(t.elapsed() < Duration::from_millis(50));
        let (ok, msg) = wait(&job).unwrap();
        assert!(!ok && msg.contains("did not answer"), "{msg}");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn reconnect_goes_through_the_link_object_of_the_daemon() {
        let Some(fake) = FakeDaemon::start(1) else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let conn = fake.client();
        assert!(reconnect_on(&conn)
            .unwrap()
            .starts_with("Reconnection requested"));
        assert_eq!(fake.reconnects(), 1);
        fake.refuse(true);
        assert!(reconnect_on(&conn).unwrap_err().contains("refused"));
        assert_eq!(fake.reconnects(), 2);
    }

    #[test]
    fn reconnect_without_daemon_fails_cleanly() {
        let Some(bus) = crate::testbus::private_bus() else {
            return;
        };
        let conn = crate::testbus::connect(&bus.addr).unwrap();
        let err = reconnect_on(&conn).unwrap_err();
        assert!(err.starts_with("daemon unreachable"), "{err}");
    }

    #[test]
    fn fn_mode_is_read_then_follows_the_toggle() {
        let f = FnMode::default();
        assert_eq!(f.mode(), None);
        f.refresh_with(|| {}, || Ok(1));
        let t = Instant::now();
        while f.mode().is_none() {
            assert!(t.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(f.mode(), Some(1));
        f.toggle_with(|| {}, || Ok(2));
        assert_eq!(
            wait(&f.job),
            Some((true, "Fn mode: F1\u{2013}F12 first".into()))
        );
        assert_eq!(f.mode(), Some(2));
        // A refused toggle keeps the mode and reports the error.
        f.toggle_with(|| {}, || Err("Fn mode not changed: dismissed".into()));
        assert_eq!(
            wait(&f.job),
            Some((false, "Fn mode not changed: dismissed".into()))
        );
        assert_eq!(f.mode(), Some(2));
    }
}
