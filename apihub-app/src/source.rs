//! Where the UI gets its state from (#62).
//!
//! Normal case: the daemon `apple-kb-monitord` owns the keyboard; this module
//! mirrors its `StateChanged` signals into the local [`Watch`] and never
//! touches the hardware. If the daemon is absent it is D-Bus-activated when
//! installed; only if that fails does the UI run the same acquisition actor
//! locally (fallback), and stops it as soon as the daemon appears, so a
//! single process reads `/dev/hidraw` at any time.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akm_core::history::{History, HistoryEntry};
use akm_core::{Snapshot, Watch};
use apple_kb_monitord::actor::{self, ActorHandle};
use apple_kb_monitord::client;
use apple_kb_monitord::service::{BUS_NAME, INTERFACE, OBJECT_PATH};
use zbus::blocking::{Connection, MessageIterator};
use zbus::MatchRule;

use crate::view::Feed;

/// Where the state currently comes from, readable from the UI thread.
#[derive(Clone, Default)]
pub struct FeedCell(Arc<AtomicU8>);

impl FeedCell {
    pub fn get(&self) -> Feed {
        match self.0.load(Ordering::Relaxed) {
            1 => Feed::Daemon,
            2 => Feed::Local,
            _ => Feed::Starting,
        }
    }

    fn set(&self, f: Feed) {
        let v = match f {
            Feed::Starting => 0,
            Feed::Daemon => 1,
            Feed::Local => 2,
        };
        self.0.store(v, Ordering::Relaxed);
    }
}

struct Source {
    watch: Arc<Watch>,
    feed: FeedCell,
    local: Option<ActorHandle>,
    /// Set by [`SourceHandle::stop`]: never start the fallback again.
    stopped: bool,
}

impl Source {
    fn go_local(&mut self) {
        if self.local.is_none() && !self.stopped {
            eprintln!("[source] daemon absent: local acquisition (fallback)");
            self.local = Some(actor::spawn(
                self.watch.clone(),
                actor::Mailbox::new(),
                actor::Options::default(),
            ));
        }
        if self.local.is_some() {
            self.feed.set(Feed::Local);
        }
    }

    fn go_daemon(&mut self, s: Snapshot) {
        if let Some(h) = self.local.take() {
            eprintln!("[source] daemon present: local acquisition stopped");
            h.stop();
        }
        self.feed.set(Feed::Daemon);
        self.watch.publish(s);
    }

    /// Align with the bus: daemon if present (or activatable), local otherwise.
    fn sync(&mut self, conn: &Connection, allow_activation: bool) {
        if !client::daemon_present(conn) && allow_activation {
            client::activate(conn);
        }
        if client::daemon_present(conn) {
            match client::fetch_snapshot(conn) {
                Ok(s) => self.go_daemon(s),
                // The daemon owns the keyboard: never open hidraw next to it
                // (#199). Show why, keep waiting for its next StateChanged.
                Err(e) => {
                    eprintln!("[source] daemon unreadable: {e}");
                    self.go_daemon(unreadable_snapshot(&self.watch.get(), &e.to_string()));
                }
            }
            return;
        }
        self.go_local();
    }
}

/// Last known state with the reason the daemon could not be read.
fn unreadable_snapshot(last: &Snapshot, err: &str) -> Snapshot {
    let mut s = last.clone();
    s.kb_error = Some(crate::i18n::trf("daemon unreadable: {}", &[&err]));
    s
}

fn subscribe(conn: &Connection) -> zbus::Result<MessageIterator> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn)?;
    dbus.add_match_rule(
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, BUS_NAME)?
            .build(),
    )?;
    dbus.add_match_rule(
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(INTERFACE)?
            .path(OBJECT_PATH)?
            .member("StateChanged")?
            .build(),
    )?;
    Ok(MessageIterator::from(conn.clone()))
}

type Shared = Arc<Mutex<Source>>;

fn lock(src: &Shared) -> std::sync::MutexGuard<'_, Source> {
    src.lock().unwrap_or_else(|e| e.into_inner())
}

fn serve_once(src: &Shared, quit: &AtomicBool) -> zbus::Result<()> {
    let conn = Connection::session()?;
    let it = subscribe(&conn)?; // subscribe first: no change lost in between
    lock(src).sync(&conn, true);
    for msg in it {
        if quit.load(Ordering::Relaxed) {
            return Ok(());
        }
        let msg = msg?;
        match msg
            .header()
            .member()
            .map(|m| m.as_str().to_string())
            .as_deref()
        {
            Some("NameOwnerChanged") => {
                // Daemon (re)started or stopped; do not re-activate a daemon
                // that was just stopped on purpose.
                lock(src).sync(&conn, false);
            }
            Some("StateChanged") => {
                if let Ok((_, json)) = msg.body().deserialize::<(u64, String)>() {
                    if let Ok(s) = serde_json::from_str::<Snapshot>(&json) {
                        lock(src).go_daemon(s);
                    }
                }
            }
            _ => {}
        }
    }
    Err(zbus::Error::Failure("session bus stream ended".into()))
}

/// Running state source; [`SourceHandle::stop`] releases the keyboard if
/// the fallback acquisition is running.
pub struct SourceHandle {
    src: Shared,
    quit: Arc<AtomicBool>,
    feed: FeedCell,
}

impl SourceHandle {
    /// Daemon or local fallback, for the alert of the window.
    pub fn feed(&self) -> FeedCell {
        self.feed.clone()
    }

    pub fn stop(self) {
        self.quit.store(true, Ordering::Relaxed);
        let local = {
            let mut g = lock(&self.src);
            g.stopped = true;
            g.local.take()
        };
        if let Some(h) = local {
            h.stop();
        }
    }
}

/// Spawn the state source thread.
pub fn spawn(watch: Arc<Watch>) -> SourceHandle {
    let feed = FeedCell::default();
    let src: Shared = Arc::new(Mutex::new(Source {
        watch,
        feed: feed.clone(),
        local: None,
        stopped: false,
    }));
    let quit = Arc::new(AtomicBool::new(false));
    let (s2, q2) = (src.clone(), quit.clone());
    let _ = std::thread::Builder::new()
        .name("state-source".into())
        .spawn(move || {
            while !q2.load(Ordering::Relaxed) {
                if let Err(e) = serve_once(&s2, &q2) {
                    eprintln!("[source] session bus: {e} — local acquisition, retry in 10s");
                    if !q2.load(Ordering::Relaxed) {
                        lock(&s2).go_local();
                    }
                }
                std::thread::sleep(Duration::from_secs(10));
            }
        });
    SourceHandle { src, quit, feed }
}

/// Battery history from the daemon, `None` when it is absent or fails.
/// Blocking D-Bus I/O: call it from a worker thread only (see
/// `history_view`, #230), never from the UI thread.
pub fn load_history_from_daemon() -> Option<Vec<HistoryEntry>> {
    let conn = Connection::session().ok()?;
    if !client::daemon_present(&conn) {
        return None;
    }
    client::fetch_history(&conn, 0)
        .map_err(|e| eprintln!("[source] History() failed: {e}"))
        .ok()
}

/// Battery history from the file (fallback). Disk I/O: worker thread only.
pub fn load_history_from_file() -> Vec<HistoryEntry> {
    History::open_default().read()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_cell_round_trips() {
        let c = FeedCell::default();
        assert_eq!(c.get(), Feed::Starting);
        for f in [Feed::Daemon, Feed::Local, Feed::Starting] {
            c.set(f);
            assert_eq!(c.clone().get(), f);
        }
    }

    #[test]
    fn unreadable_daemon_keeps_last_state_and_says_why() {
        let last = Snapshot {
            caps_lock: true,
            ..Default::default()
        };
        let s = unreadable_snapshot(&last, "bad Json property");
        assert!(s.caps_lock);
        assert_eq!(
            s.kb_error.as_deref(),
            Some("daemon unreadable: bad Json property")
        );
    }
}
