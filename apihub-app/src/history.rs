//! Battery history: thin wrapper over `akm_core::history` (XDG state dir,
//! one-time migration from `~/.local/share`, 90-day rotation).

use std::sync::Once;

pub use akm_core::history::HistoryEntry;
use akm_core::history::{legacy_path, History, RETENTION_S};

fn store() -> History {
    static MIGRATE: Once = Once::new();
    let h = History::open_default();
    MIGRATE.call_once(|| {
        if let Err(e) = h.migrate_from(&legacy_path()) {
            eprintln!("[history] migration failed: {e}");
        }
        if let Err(e) = h.rotate(RETENTION_S) {
            eprintln!("[history] rotation failed: {e}");
        }
    });
    h
}

/// Append a sample (best effort; invalid samples are ignored).
pub fn append_history(pct: f64, voltage: Option<f64>) {
    if let Err(e) = store().append(pct, voltage) {
        eprintln!("[history] append failed: {e}");
    }
}

/// (rate mV/h, remaining hours) from the stored history.
pub fn estimate_remaining() -> Option<(f64, f64)> {
    store().estimate_remaining()
}

/// Every stored entry.
pub fn read_history() -> Vec<HistoryEntry> {
    store().read()
}
