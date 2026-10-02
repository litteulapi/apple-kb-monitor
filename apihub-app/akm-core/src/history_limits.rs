//! Bounds of the battery history (#96): the size of the file on disk and the
//! number of points handed to a client.
//!
//! * **Size**: `history.jsonl` never stays above [`MAX_BYTES`] (5 MiB). When
//!   it does, the oldest ordinary samples are dropped until the file is back
//!   under [`TRIM_TO_BYTES`], in one atomic rewrite, after a `.prev` copy (the
//!   same safety net as the age rotation). Lines carrying an event (battery
//!   replacement) are kept whatever their age: the lifetime of every set of
//!   batteries stays known. At one sample per 5 minutes and about 110 bytes a
//!   line, 5 MiB is more than the 90-day retention: the bound only bites on a
//!   file that grew abnormally.
//! * **Points**: [`downsample`] thins a series to at most `max` points for a
//!   D-Bus client (`History(since)`), keeping the first point, the last one,
//!   every event and, in between, evenly spaced samples.

use std::io::{self, Write};

use crate::history::{parse_bytes, Clock, History, HistoryEntry};

/// Largest size of `history.jsonl` (5 MiB).
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Size the file is brought back to when it went over [`MAX_BYTES`] (80 %):
/// the next rewrite is far away, not at the next sample.
pub const TRIM_TO_BYTES: u64 = MAX_BYTES / 5 * 4;
/// Points returned by `History(since)` when the client gives no bound.
pub const DEFAULT_MAX_POINTS: usize = 2000;
/// Largest bound a client may ask for.
pub const MAX_POINTS_LIMIT: usize = 20_000;

/// At most `max` entries of `entries` (chronological order kept): the first,
/// the last, every entry carrying an event, and evenly spaced samples in
/// between. `max == 0` means [`DEFAULT_MAX_POINTS`]; a bound above
/// [`MAX_POINTS_LIMIT`] is brought back to it. A series already within the
/// bound comes back untouched.
pub fn downsample(entries: Vec<HistoryEntry>, max: usize) -> Vec<HistoryEntry> {
    let max = match max {
        0 => DEFAULT_MAX_POINTS,
        m => m.min(MAX_POINTS_LIMIT),
    };
    let n = entries.len();
    if n <= max {
        return entries;
    }
    let mut keep = vec![false; n];
    // Ends first, then events (newest first: the current set matters most).
    let mandatory = [n - 1, 0]
        .into_iter()
        .chain((0..n).rev().filter(|i| entries[*i].event.is_some()));
    let mut kept = 0usize;
    for i in mandatory {
        if kept < max && !keep[i] {
            keep[i] = true;
            kept += 1;
        }
    }
    // What is left of the budget: evenly spaced among the other samples.
    let budget = max - kept;
    let others: Vec<usize> = (0..n).filter(|i| !keep[*i]).collect();
    let c = others.len();
    for k in 0..budget {
        // Centre of the k-th of `budget` equal slices: distinct as c >= budget.
        keep[others[(2 * k + 1) * c / (2 * budget)]] = true;
    }
    entries
        .into_iter()
        .zip(keep)
        .filter_map(|(e, k)| k.then_some(e))
        .collect()
}

fn line_len(e: &HistoryEntry) -> io::Result<u64> {
    Ok(serde_json::to_string(e).map_err(io::Error::other)?.len() as u64 + 1)
}

impl<C: Clock> History<C> {
    /// Entries with `ts >= since`, thinned to at most `max` points
    /// ([`downsample`]).
    pub fn read_since_bounded(&self, since: u64, max: usize) -> Vec<HistoryEntry> {
        downsample(self.read_since(since), max)
    }

    /// Size of the file on disk (0 if missing).
    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(self.path()).map(|m| m.len()).unwrap_or(0)
    }

    /// Bring the file back under `trim_to` bytes when it is above `max_bytes`
    /// (see the module). Returns the number of lines removed (0 = nothing
    /// done). Lines that do not parse are dropped by the rewrite, after being
    /// appended to `history.jsonl.corrupt` like the age rotation does.
    pub fn enforce_size_with(&self, max_bytes: u64, trim_to: u64) -> io::Result<usize> {
        if self.size_bytes() <= max_bytes {
            return Ok(0);
        }
        let content = std::fs::read(self.path())?;
        let total = content
            .split(|b| *b == b'\n')
            .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
            .count();
        let rejected: Vec<&[u8]> = content
            .split(|b| *b == b'\n')
            .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
            .filter(|l| parse_bytes(l).is_empty())
            .collect();
        let mut entries = parse_bytes(&content);
        entries.sort_by_key(|e| e.ts);
        let mut size = 0u64;
        for e in &entries {
            size += line_len(e)?;
        }
        // Oldest ordinary samples go first; events stay.
        let mut drop = vec![false; entries.len()];
        for (i, e) in entries.iter().enumerate() {
            if size <= trim_to {
                break;
            }
            if e.event.is_none() {
                drop[i] = true;
                size -= line_len(e)?;
            }
        }
        let keep: Vec<&HistoryEntry> = entries
            .iter()
            .zip(&drop)
            .filter_map(|(e, d)| (!d).then_some(e))
            .collect();
        if !rejected.is_empty() {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path().with_extension("jsonl.corrupt"))?;
            let mut buf = Vec::new();
            for l in &rejected {
                buf.extend_from_slice(l);
                buf.push(b'\n');
            }
            f.write_all(&buf)?;
            f.sync_all()?;
        }
        std::fs::copy(self.path(), self.path().with_extension("jsonl.prev"))?;
        let tmp = self.path().with_extension("jsonl.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            let mut out = Vec::with_capacity(size as usize);
            for e in &keep {
                out.extend_from_slice(
                    serde_json::to_string(e)
                        .map_err(io::Error::other)?
                        .as_bytes(),
                );
                out.push(b'\n');
            }
            f.write_all(&out)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, self.path())?;
        Ok(total - keep.len())
    }

    /// [`Self::enforce_size_with`] with the limits of the daemon
    /// ([`MAX_BYTES`], [`TRIM_TO_BYTES`]).
    pub fn enforce_size(&self) -> io::Result<usize> {
        self.enforce_size_with(MAX_BYTES, TRIM_TO_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEvent, SystemClock};
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Tmp(std::path::PathBuf);
    impl Tmp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "akm-histlim-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const T0: u64 = 1_790_000_000;

    fn series(n: u64) -> Vec<HistoryEntry> {
        (0..n)
            .map(|i| HistoryEntry::measured(T0 + i * 300, 80.0, Some(2800), Some(2763)))
            .collect()
    }

    #[test]
    fn the_limits_are_the_documented_ones() {
        assert_eq!(MAX_BYTES, 5 * 1024 * 1024);
        const { assert!(TRIM_TO_BYTES < MAX_BYTES) };
        // 90 days at one sample per 5 minutes fit under the bound.
        let line = line_len(&series(1)[0]).unwrap();
        assert!(90 * 288 * line < MAX_BYTES, "{line} bytes a line");
    }

    #[test]
    fn a_series_within_the_bound_is_untouched() {
        let s = series(50);
        assert_eq!(downsample(s.clone(), 50), s);
        assert_eq!(downsample(s.clone(), 0), s, "0 = default bound");
        assert_eq!(downsample(Vec::new(), 10), Vec::new());
    }

    #[test]
    fn a_long_series_is_thinned_keeping_the_ends_the_events_and_the_order() {
        let mut s = series(25_920); // 90 days
        s[100].event = Some(HistoryEvent::BatteryReplaced);
        s[20_000].event = Some(HistoryEvent::BatteryReplaced);
        let d = downsample(s.clone(), 500);
        assert!(d.len() <= 500 && d.len() >= 490, "{}", d.len());
        assert_eq!(d.first(), s.first());
        assert_eq!(d.last(), s.last());
        assert_eq!(d.iter().filter(|e| e.event.is_some()).count(), 2);
        assert!(d.windows(2).all(|w| w[0].ts < w[1].ts), "chronological");
        // Evenly spread: no hole larger than three times the mean step.
        let step = (s.last().unwrap().ts - s[0].ts) / d.len() as u64;
        assert!(d.windows(2).all(|w| w[1].ts - w[0].ts <= 3 * step));
        // The default bound, and the ceiling of what a client may ask.
        assert_eq!(downsample(s.clone(), 0).len(), DEFAULT_MAX_POINTS);
        assert_eq!(downsample(s.clone(), usize::MAX).len(), MAX_POINTS_LIMIT);
        // A bound smaller than the number of events never exceeds the bound.
        assert_eq!(downsample(s, 1).len(), 1);
    }

    #[test]
    fn a_file_over_the_bound_is_rotated_under_it_and_events_survive() {
        let t = Tmp::new();
        let h = History::new(t.0.join("h.jsonl"), SystemClock);
        let mut s = series(2_000);
        s[3].event = Some(HistoryEvent::BatteryReplaced);
        for e in &s {
            h.append_entry(e).unwrap();
        }
        let size = h.size_bytes();
        // Under the bound: nothing happens, no rewrite.
        assert_eq!(h.enforce_size_with(size, size / 2).unwrap(), 0);
        assert!(!t.0.join("h.jsonl.prev").exists());
        // Over it: the oldest samples go, the file ends under the target.
        let removed = h.enforce_size_with(size - 1, size / 2).unwrap();
        assert!((990..=1_010).contains(&removed), "{removed}");
        assert!(
            h.size_bytes() <= size / 2,
            "{} > {}",
            h.size_bytes(),
            size / 2
        );
        let left = h.read();
        assert_eq!(left.len(), 2_000 - removed);
        assert_eq!(left.last(), s.last(), "the newest sample is kept");
        assert_eq!(
            left.iter().filter(|e| e.event.is_some()).count(),
            1,
            "the battery replacement outlives the size rotation"
        );
        assert_eq!(left[0].ts, s[3].ts);
        assert!(left[1].ts > s[900].ts, "the oldest ordinary samples went");
        assert_eq!(
            std::fs::metadata(t.0.join("h.jsonl.prev")).unwrap().len(),
            size,
            "one generation of safety net"
        );
        // Idempotent.
        assert_eq!(h.enforce_size_with(size - 1, size / 2).unwrap(), 0);
    }

    #[test]
    fn corrupt_lines_are_set_aside_by_the_size_rotation() {
        let t = Tmp::new();
        let h = History::new(t.0.join("h.jsonl"), SystemClock);
        for e in &series(20) {
            h.append_entry(e).unwrap();
        }
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(h.path())
            .unwrap();
        f.write_all(b"{not json\n").unwrap();
        let size = h.size_bytes();
        let removed = h.enforce_size_with(size - 1, size).unwrap();
        assert_eq!(removed, 1, "only the corrupt line");
        assert_eq!(h.read().len(), 20);
        let corrupt = std::fs::read_to_string(t.0.join("h.jsonl.corrupt")).unwrap();
        assert_eq!(corrupt, "{not json\n");
    }

    #[test]
    fn read_since_bounded_filters_then_thins() {
        let t = Tmp::new();
        let h = History::new(t.0.join("h.jsonl"), SystemClock);
        for e in &series(300) {
            h.append_entry(e).unwrap();
        }
        let d = h.read_since_bounded(T0 + 100 * 300, 20);
        assert_eq!(d.len(), 20);
        assert_eq!(d[0].ts, T0 + 100 * 300);
        assert_eq!(d[19].ts, T0 + 299 * 300);
        assert_eq!(h.read_since_bounded(T0 + 290 * 300, 20).len(), 10);
    }
}
