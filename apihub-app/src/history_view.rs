//! Battery history for the window, loaded **off the UI thread** (#230).
//!
//! 3.1.0 called the daemon's `History()` (a blocking D-Bus call without
//! timeout) from `App::new` and from the Refresh button, on the thread that
//! answers the compositor's pings: a slow or stuck daemon froze the window
//! ("not responding"). Here the UI only reads [`Shared`]; a worker thread
//! does the I/O, waits for the daemon at most [`DAEMON_TIMEOUT`], then falls
//! back to the history file. At most one load and one daemon call run at a
//! time, so repeated clicks never pile up threads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use akm_core::history::HistoryEntry;

/// Longest wait for the daemon's `History()` reply before reading the file.
pub const DAEMON_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default)]
pub struct Data {
    pub battery: Series,
    pub voltage: Series,
    pub loading: bool,
    /// Where the last load came from, or why the daemon was skipped.
    pub note: Option<String>,
}

pub type Shared = Arc<Mutex<Data>>;

/// (timestamp, value) points of one chart series.
pub type Series = Vec<(f64, f64)>;

pub fn series(entries: &[HistoryEntry]) -> (Series, Series) {
    let battery = entries.iter().map(|e| (e.ts as f64, e.pct)).collect();
    let voltage = entries
        .iter()
        .filter_map(|e| e.reliable_voltage().map(|v| (e.ts as f64, v)))
        .collect();
    (battery, voltage)
}

pub struct Loader {
    data: Shared,
    busy: Arc<AtomicBool>,
    daemon_call: Arc<AtomicBool>,
}

impl Loader {
    pub fn new() -> Self {
        Self {
            data: Shared::default(),
            busy: Arc::new(AtomicBool::new(false)),
            daemon_call: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn data(&self) -> Data {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Start a load unless one is running; `repaint` is called when done.
    pub fn request(&self, repaint: impl Fn() + Send + 'static) {
        self.request_with(
            repaint,
            crate::source::load_history_from_daemon,
            crate::source::load_history_from_file,
        );
    }

    pub fn request_with(
        &self,
        repaint: impl Fn() + Send + 'static,
        daemon: impl FnOnce() -> Option<Vec<HistoryEntry>> + Send + 'static,
        file: impl FnOnce() -> Vec<HistoryEntry> + Send + 'static,
    ) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        self.data.lock().unwrap_or_else(|e| e.into_inner()).loading = true;
        let (data, busy, daemon_call) = (
            self.data.clone(),
            self.busy.clone(),
            self.daemon_call.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("history-load".into())
            .spawn(move || {
                let (entries, note) = load_bounded(&daemon_call, daemon, file);
                let (battery, voltage) = series(&entries);
                *data.lock().unwrap_or_else(|e| e.into_inner()) = Data {
                    battery,
                    voltage,
                    loading: false,
                    note,
                };
                busy.store(false, Ordering::Release);
                repaint();
            });
        if spawned.is_err() {
            self.data.lock().unwrap_or_else(|e| e.into_inner()).loading = false;
            self.busy.store(false, Ordering::Release);
        }
    }
}

/// Daemon first (bounded wait, never two calls in flight), file otherwise.
fn load_bounded(
    daemon_call: &Arc<AtomicBool>,
    daemon: impl FnOnce() -> Option<Vec<HistoryEntry>> + Send + 'static,
    file: impl FnOnce() -> Vec<HistoryEntry>,
) -> (Vec<HistoryEntry>, Option<String>) {
    if daemon_call.swap(true, Ordering::AcqRel) {
        return (
            file(),
            Some(crate::i18n::tr("daemon still busy: history read from the file").into()),
        );
    }
    let (tx, rx) = mpsc::channel();
    let flag = daemon_call.clone();
    let spawned = std::thread::Builder::new()
        .name("history-dbus".into())
        .spawn(move || {
            let r = daemon();
            flag.store(false, Ordering::Release);
            let _ = tx.send(r);
        });
    if spawned.is_err() {
        daemon_call.store(false, Ordering::Release);
        return (file(), None);
    }
    match rx.recv_timeout(DAEMON_TIMEOUT) {
        Ok(Some(h)) => (h, None),
        Ok(None) => (file(), None),
        Err(_) => (
            file(),
            Some(crate::i18n::trf(
                "daemon did not answer within {} s: history read from the file",
                &[&DAEMON_TIMEOUT.as_secs()],
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn entry(ts: u64, pct: f64) -> HistoryEntry {
        serde_json::from_str(&format!(r#"{{"ts":{ts},"pct":{pct}}}"#)).unwrap()
    }

    fn wait_loaded(l: &Loader) -> Data {
        let t = Instant::now();
        loop {
            let d = l.data();
            if !d.loading {
                return d;
            }
            assert!(t.elapsed() < DAEMON_TIMEOUT * 3, "load never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The UI-side call returns at once even when the daemon never answers,
    /// and the file is used after the bounded wait (#230).
    #[test]
    fn a_stuck_daemon_never_blocks_the_caller() {
        let l = Loader::new();
        let t = Instant::now();
        l.request_with(
            || {},
            || {
                std::thread::sleep(Duration::from_secs(3600));
                None
            },
            || vec![entry(1, 50.0), entry(2, 51.0)],
        );
        assert!(
            t.elapsed() < Duration::from_millis(50),
            "request blocked {:?}",
            t.elapsed()
        );
        assert!(l.data().loading);
        let d = wait_loaded(&l);
        assert!(t.elapsed() >= DAEMON_TIMEOUT && t.elapsed() < DAEMON_TIMEOUT * 2);
        assert_eq!(d.battery, vec![(1.0, 50.0), (2.0, 51.0)]);
        assert!(d.note.unwrap().contains("did not answer"));
        // The stuck call is still in flight: the next load does not wait again.
        let t = Instant::now();
        l.request_with(
            || {},
            || unreachable!("second daemon call"),
            || vec![entry(3, 60.0)],
        );
        let d = wait_loaded(&l);
        assert!(t.elapsed() < Duration::from_secs(1));
        assert_eq!(d.battery, vec![(3.0, 60.0)]);
    }

    #[test]
    fn daemon_answer_is_used_and_clicks_do_not_pile_up() {
        let l = Loader::new();
        l.request_with(
            || {},
            || {
                std::thread::sleep(Duration::from_millis(200));
                Some(vec![entry(5, 70.0)])
            },
            Vec::new,
        );
        // Second click while loading: ignored (no second thread, no second call).
        l.request_with(
            || {},
            || unreachable!("concurrent load"),
            || unreachable!("concurrent load"),
        );
        let d = wait_loaded(&l);
        assert_eq!(d.battery, vec![(5.0, 70.0)]);
        assert_eq!(d.note, None);
    }
}
