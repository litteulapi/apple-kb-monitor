//! Battery history logging — JSONL append-only store.
//!
//! Each entry records a timestamp, battery percentage, and voltage.
//! Stored in `~/.local/share/apple-kb-monitor/history.jsonl`.

use serde::{Deserialize, Serialize};

const HISTORY_FILE: &str = "apple-kb-monitor/history.jsonl";

/// A single battery history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: u64,
    pub pct: f64,
    pub voltage: f64,
}

/// Return the path to the history JSONL file.
fn history_path() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(HISTORY_FILE)
}

/// Append a battery reading to the history log.
///
/// Creates the parent directory if needed. Silently ignores write errors
/// (the history file is best-effort, not critical).
pub fn append_history(pct: f64, voltage: f64) {
    let ts = unsafe { libc::time(std::ptr::null_mut()) } as u64;
    let entry = HistoryEntry { ts, pct, voltage };

    let path = history_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Ok(line) = serde_json::to_string(&entry) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{}", line);
        }
    }
}

/// Estimate battery discharge rate and remaining time from history.
/// Returns (rate_mv_per_hour, remaining_hours) or None if insufficient data.
pub fn estimate_remaining() -> Option<(f64, f64)> {
    let entries = read_history();
    if entries.len() < 2 { return None; }

    // Use last 50 entries
    let recent: Vec<&HistoryEntry> = entries.iter().rev().take(50).collect();
    if recent.len() < 2 { return None; }

    let first = recent.last().unwrap();
    let last = recent.first().unwrap();
    let hours = (last.ts as f64 - first.ts as f64) / 3600.0;
    if hours < 0.01 { return None; }

    let rate_mvh = (first.voltage - last.voltage) * 1000.0 / hours;
    if rate_mvh < 0.1 { return None; } // not discharging

    let remaining_mv = (last.voltage - 2.0) * 1000.0;
    if remaining_mv <= 0.0 { return Some((rate_mvh, 0.0)); }

    let remaining_hours = remaining_mv / rate_mvh;
    Some((rate_mvh, remaining_hours))
}

/// Read all history entries from disk.
///
/// Returns an empty vec on any I/O or parse error. Individual malformed
/// lines are silently skipped.
pub fn read_history() -> Vec<HistoryEntry> {
    let path = history_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| serde_json::from_str::<HistoryEntry>(line).ok())
        .collect()
}
