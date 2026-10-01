//! Battery history as seen by the UI. The acquisition (daemon, or the local
//! actor when the daemon is absent) is the single writer.

pub use akm_core::history::HistoryEntry;

/// Every stored entry, read from the history file.
pub fn read_history() -> Vec<HistoryEntry> {
    akm_core::history::History::open_default().read()
}
