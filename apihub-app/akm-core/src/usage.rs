//! Usage statistics without any key logging: how many minutes per day the keyboard was used.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Days kept; older counters are dropped.
pub const KEEP_DAYS: usize = 90;
/// Minutes in a day: no counter goes above.
pub const DAY_MINUTES: u16 = 1440;

/// Active minutes per local day (`YYYY-MM-DD`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStats {
    #[serde(default)]
    pub days: BTreeMap<String, u16>,
    #[serde(skip)]
    last_minute: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayUsage {
    /// Local day, `YYYY-MM-DD`.
    pub day: String,
    pub active_minutes: u16,
    /// `active_minutes / 60`, one decimal.
    pub active_hours: f64,
}

/// What `GetState` carries (`usage`), when the statistics are enabled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSummary {
    pub today_active_minutes: u16,
    /// The last 7 days, oldest first, today last (days without use: 0).
    pub days: Vec<DayUsage>,
}

fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

impl UsageStats {
    /// `$XDG_STATE_HOME/apple-kb-monitor/usage.json`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        crate::history::default_path().with_file_name("usage.json")
    }

    /// An unreadable or corrupt file is an empty record.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let mut s: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.days.retain(|d, m| is_day(d) && *m <= DAY_MINUTES);
        s.trim();
        s
    }

    /// Atomic write (temporary file + rename), mode 0600.
    ///
    /// # Errors
    ///
    /// Any I/O error while creating, writing or renaming the file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::fsutil::write_atomic(path, json.as_bytes(), crate::fsutil::Mode::Fixed(0o600))
    }

    fn trim(&mut self) {
        while self.days.len() > KEEP_DAYS {
            let Some(oldest) = self.days.keys().next().cloned() else {
                break;
            };
            self.days.remove(&oldest);
        }
    }

    /// The keyboard sent something at unix time `now`, on local day `day`.
    pub fn note(&mut self, now: u64, day: &str) -> bool {
        let minute = now / 60;
        if self.last_minute == Some(minute) || !is_day(day) {
            return false;
        }
        self.last_minute = Some(minute);
        let m = self.days.entry(day.to_string()).or_insert(0);
        if *m >= DAY_MINUTES {
            return false;
        }
        *m += 1;
        self.trim();
        true
    }

    #[must_use]
    pub fn minutes(&self, day: &str) -> u16 {
        self.days.get(day).copied().unwrap_or(0)
    }

    /// The 7 days ending at the day of unix time `now` (local time).
    #[must_use]
    pub fn summary(&self, now: u64) -> UsageSummary {
        let days: Vec<DayUsage> = (0..7u64)
            .rev()
            .map(|back| {
                let day = local_day(now.saturating_sub(back * 86_400));
                let active_minutes = self.minutes(&day);
                DayUsage {
                    day,
                    active_minutes,
                    active_hours: (f64::from(active_minutes) / 6.0).round() / 10.0,
                }
            })
            .collect();
        UsageSummary {
            today_active_minutes: days.last().map_or(0, |d| d.active_minutes),
            days,
        }
    }
}

/// Local day `YYYY-MM-DD` of unix time `ts` (UTC if the conversion fails).
#[must_use]
pub fn local_day(ts: u64) -> String {
    let Some(tm) = crate::conv::local_tm(ts) else {
        return crate::history::format_utc(ts)[..10].to_string();
    };
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_790_944_440; // a whole minute

    #[test]
    fn a_minute_with_any_number_of_events_counts_once() {
        let mut u = UsageStats::default();
        assert!(u.note(T0, "2026-10-02"));
        for s in 1..60 {
            assert!(!u.note(T0 + s, "2026-10-02"), "same minute");
        }
        assert!(u.note(T0 + 60, "2026-10-02"));
        assert_eq!(u.minutes("2026-10-02"), 2);
        for m in 2..62 {
            u.note(T0 + m * 60 + 30, "2026-10-02");
        }
        assert_eq!(u.minutes("2026-10-02"), 62);
        assert_eq!(u.minutes("2026-10-03"), 0);
        assert!(u.note(T0 + 86_400, "2026-10-03"));
        assert_eq!((u.minutes("2026-10-02"), u.minutes("2026-10-03")), (62, 1));
    }

    #[test]
    fn a_day_is_bounded_and_the_record_keeps_ninety_days() {
        let mut u = UsageStats::default();
        for m in 0..2_000u64 {
            u.note(T0 + m * 60, "2026-10-02");
        }
        assert_eq!(u.minutes("2026-10-02"), DAY_MINUTES);
        let mut u = UsageStats::default();
        for d in 0..120u64 {
            let day = format!("2026-{:02}-{:02}", 1 + d / 28, 1 + d % 28);
            u.note(T0 + d * 86_400, &day);
        }
        assert_eq!(u.days.len(), KEEP_DAYS);
        assert!(!u.days.contains_key("2026-01-01"), "the oldest days went");
        let mut u = UsageStats::default();
        assert!(!u.note(T0, "today"));
        assert!(u.days.is_empty());
    }

    #[test]
    fn only_a_counter_per_day_is_stored() {
        let dir = std::env::temp_dir().join(format!("akm-usage-{}", std::process::id()));
        let path = dir.join("usage.json");
        let mut u = UsageStats::default();
        for m in 0..5u64 {
            u.note(T0 + m * 60, "2026-10-02");
        }
        u.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v, serde_json::json!({"days": {"2026-10-02": 5}}));
        assert!(!text.contains(&(T0 / 60).to_string()), "no time of day");
        assert_eq!(UsageStats::load(&path).minutes("2026-10-02"), 5);
        // A corrupt or hand-edited file never brings anything else in.
        std::fs::write(
            &path,
            r#"{"days":{"2026-10-02":7,"keys":3,"2026-10-03":9999},"x":1}"#,
        )
        .unwrap();
        let back = UsageStats::load(&path);
        assert_eq!(back.days.len(), 1);
        assert_eq!(back.minutes("2026-10-02"), 7);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(UsageStats::load(&path), UsageStats::default());
        let _ = std::fs::remove_dir_all(&dir);
        let _: fn(&mut UsageStats, u64, &str) -> bool = UsageStats::note;
    }

    #[allow(clippy::float_cmp)] // reason: exact values are the property under test
    #[test]
    fn the_summary_is_seven_days_ending_today() {
        let now = T0;
        let today = local_day(now);
        let yesterday = local_day(now - 86_400);
        let mut u = UsageStats::default();
        for m in 0..90u64 {
            u.note(now + m * 60, &today);
        }
        u.days.insert(yesterday.clone(), 30);
        let s = u.summary(now);
        assert_eq!(s.days.len(), 7);
        assert_eq!(s.today_active_minutes, 90);
        assert_eq!(s.days[6].day, today);
        assert_eq!(s.days[6].active_hours, 1.5);
        assert_eq!(
            (s.days[5].day.as_str(), s.days[5].active_minutes),
            (yesterday.as_str(), 30)
        );
        assert_eq!(
            s.days[0].active_minutes, 0,
            "a day without use is 0, not absent"
        );
        assert!(is_day(&today));
    }
}
