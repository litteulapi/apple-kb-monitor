//! Link quality of the daemon.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use akm_core::linkstats::{LinkQuality, LinkStats};

/// Signal measurements alone trigger a save at most this often (seconds).
pub const RSSI_SAVE_EVERY: u64 = 600;
/// A reason arriving this long after `Connected = false` still refines it.
pub const REFINE_WITHIN_S: u64 = 5;

#[derive(Debug)]
struct Inner {
    stats: LinkStats,
    last_save: u64,
}

/// The link statistics of one daemon.
#[derive(Debug)]
pub struct Store {
    inner: Mutex<Inner>,
    path: Option<PathBuf>,
}

impl Store {
    pub fn new(path: Option<PathBuf>) -> Arc<Self> {
        let stats = path.as_deref().map(LinkStats::load).unwrap_or_default();
        Arc::new(Self {
            inner: Mutex::new(Inner {
                stats,
                last_save: 0,
            }),
            path,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn save(&self, g: &mut Inner, now: u64) {
        g.last_save = now;
        if let Some(p) = self.path.as_deref() {
            if let Err(e) = g.stats.save(p) {
                tracing::warn!("cannot save {}: {e}", p.display());
            }
        }
    }

    pub fn disconnect(&self, mac: &str, now: u64, reason: &str) {
        let mut g = self.lock();
        g.stats.record_disconnect(mac, now, reason);
        self.save(&mut g, now);
    }

    /// The reason of the disconnection just recorded is now known.
    pub fn refine(&self, mac: &str, now: u64, reason: &str) {
        let mut g = self.lock();
        if g.stats.refine_last(mac, now, reason, REFINE_WITHIN_S) {
            self.save(&mut g, now);
        }
    }

    /// A signal measurement (relative dB).
    pub fn rssi(&self, mac: &str, now: u64, rel_db: i32) {
        let mut g = self.lock();
        g.stats.record_rssi(mac, now, rel_db);
        if now.saturating_sub(g.last_save) >= RSSI_SAVE_EVERY {
            self.save(&mut g, now);
        }
    }

    /// Is the "unstable link" alert due?
    pub fn unstable_due(&self, mac: &str, now: u64) -> Option<usize> {
        let mut g = self.lock();
        let before = g.stats.quality(mac, now).and_then(|q| q.unstable_since);
        let due = g.stats.unstable_due(mac, now);
        if g.stats.quality(mac, now).and_then(|q| q.unstable_since) != before {
            self.save(&mut g, now);
        }
        due
    }

    /// Summary of `mac` for `GetState` and `Link.Status()`.
    pub fn quality(&self, mac: &str, now: u64) -> Option<LinkQuality> {
        self.lock().stats.quality(mac, now)
    }

    /// Everything kept plus the summary of each keyboard: `Link.Quality()`.
    pub fn json(&self, now: u64) -> serde_json::Value {
        let g = self.lock();
        serde_json::Value::Object(
            g.stats
                .devices
                .iter()
                .map(|(mac, d)| {
                    (
                        mac.clone(),
                        serde_json::json!({
                            "summary": d.quality(now),
                            "disconnects": d.disconnects,
                            "signal_by_hour": d
                                .rssi
                                .iter()
                                .map(|h| serde_json::json!({
                                    "start": h.start, "samples": h.samples,
                                    "mean": h.mean(), "min": h.min, "max": h.max,
                                }))
                                .collect::<Vec<_>>(),
                        }),
                    )
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "AA:BB:CC:DD:EE:F1";
    const T0: u64 = 1_790_000_000;

    #[test]
    fn disconnections_are_saved_at_once_and_the_signal_sparingly() {
        let dir = std::env::temp_dir().join(format!("akm-linkq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("link-quality.json");
        let s = Store::new(Some(path.clone()));
        s.disconnect(MAC, T0, "unknown");
        s.refine(MAC, T0 + 1, "timeout");
        assert_eq!(
            LinkStats::load(&path).devices[MAC].disconnects[0].reason,
            "timeout"
        );
        s.rssi(MAC, T0 + 10, -3);
        assert!(
            LinkStats::load(&path).devices[MAC].rssi.is_empty(),
            "not yet"
        );
        s.rssi(MAC, T0 + RSSI_SAVE_EVERY + 1, -5);
        assert_eq!(LinkStats::load(&path).devices[MAC].rssi[0].samples, 2);
        let again = Store::new(Some(path));
        let q = again.quality(MAC, T0 + 20).unwrap();
        assert_eq!(q.disconnects_last_hour, 1);
        let j = again.json(T0 + 20);
        assert_eq!(j[MAC]["disconnects"][0]["reason"], "timeout");
        assert_eq!(j[MAC]["signal_by_hour"][0]["mean"], -4.0);
        assert_eq!(j[MAC]["summary"]["disconnects_last_day"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_alert_is_due_once_per_episode() {
        let s = Store::new(None);
        for i in 0..4 {
            s.disconnect(MAC, T0 + i * 60, "timeout");
        }
        assert_eq!(s.unstable_due(MAC, T0 + 200), Some(4));
        assert_eq!(s.unstable_due(MAC, T0 + 260), None);
        assert!(s.quality(MAC, T0 + 260).unwrap().unstable);
        assert_eq!(s.unstable_due(MAC, T0 + 2 * 3600), None);
        assert!(!s.quality(MAC, T0 + 2 * 3600).unwrap().unstable);
    }
}
