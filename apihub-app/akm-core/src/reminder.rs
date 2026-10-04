//! "Change the batteries" and "firmware update known" notices.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::registry::{ThresholdLevel, Thresholds};

/// The voltage must climb this far above a threshold to re-arm its reminder.
pub const REARM_MARGIN_MV: u32 = 50;

/// Which threshold of the keyboard was crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderLevel {
    Low,
    Critical,
}

impl ReminderLevel {
    /// The keyboard threshold (mV) of this level.
    #[must_use]
    pub fn threshold_mv(self, t: &Thresholds) -> u16 {
        match self {
            Self::Low => t.low_mv,
            Self::Critical => t.critical_mv,
        }
    }
}

/// A "change the batteries" reminder that is due now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryReminder {
    pub level: ReminderLevel,
    /// Smoothed voltage that crossed it.
    pub mv: u32,
    /// The keyboard's threshold for this level.
    pub threshold_mv: u16,
}

/// What was already notified, per keyboard (MAC upper case).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeMemory {
    /// Deepest battery level already notified and not re-armed yet.
    #[serde(default)]
    pub battery: BTreeMap<String, ReminderLevel>,
    /// Latest firmware version (`0x0050`) already announced.
    #[serde(default)]
    pub firmware: BTreeMap<String, String>,
}

impl NoticeMemory {
    /// `$XDG_STATE_HOME/apple-kb-monitor/notices.json`, next to the history.
    #[must_use]
    pub fn default_path() -> PathBuf {
        crate::history::default_path().with_file_name("notices.json")
    }

    /// An unreadable or corrupt file is an empty memory (worst case: one notice shown again).
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
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

    /// Battery reminder for keyboard `mac` at smoothed voltage `mv`, against its own thresholds.
    pub fn battery(
        &mut self,
        mac: &str,
        mv: u32,
        t: &Thresholds,
    ) -> (Option<BatteryReminder>, bool) {
        let key = mac.to_ascii_uppercase();
        let before = self.battery.get(&key).copied();
        // Re-arm first: a voltage well above a threshold clears it.
        let mut done = before;
        if done == Some(ReminderLevel::Critical) && mv >= u32::from(t.critical_mv) + REARM_MARGIN_MV
        {
            done = Some(ReminderLevel::Low);
        }
        if done == Some(ReminderLevel::Low) && mv >= u32::from(t.low_mv) + REARM_MARGIN_MV {
            done = None;
        }
        let now = match t.level(mv) {
            ThresholdLevel::Ok => None,
            ThresholdLevel::Low => Some(ReminderLevel::Low),
            ThresholdLevel::Critical | ThresholdLevel::Empty => Some(ReminderLevel::Critical),
        };
        let due = now.filter(|n| done.is_none_or(|d| *n > d));
        if let Some(level) = due {
            done = Some(level);
        }
        match done {
            Some(d) => self.battery.insert(key, d),
            None => self.battery.remove(&key),
        };
        let reminder = due.map(|level| BatteryReminder {
            level,
            mv,
            threshold_mv: level.threshold_mv(t),
        });
        (reminder, done != before)
    }

    /// New batteries: every battery reminder of `mac` is re-armed.
    pub fn rearm_battery(&mut self, mac: &str) -> bool {
        self.battery.remove(&mac.to_ascii_uppercase()).is_some()
    }

    /// Firmware notice for keyboard `mac`.
    pub fn firmware(&mut self, mac: &str, latest: Option<&str>, up_to_date: bool) -> (bool, bool) {
        let key = mac.to_ascii_uppercase();
        match latest {
            Some(l) if self.firmware.get(&key).map(String::as_str) != Some(l) => {
                self.firmware.insert(key, l.to_string());
                (true, true)
            }
            // Up to date again (updated): the next newer version is announced.
            None if up_to_date => (false, self.firmware.remove(&key).is_some()),
            Some(_) | None => (false, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "aa:bb:cc:dd:ee:f1";
    const T: Thresholds = Thresholds {
        full_mv: 2954,
        low_mv: 2506,
        critical_mv: 2404,
        empty_mv: 2054,
    };

    fn levels(m: &mut NoticeMemory, mvs: &[u32]) -> Vec<Option<ReminderLevel>> {
        mvs.iter()
            .map(|&mv| m.battery(MAC, mv, &T).0.map(|r| r.level))
            .collect()
    }

    #[test]
    fn once_per_crossing_of_the_keyboard_thresholds() {
        use ReminderLevel::{Critical as C, Low as L};
        let mut m = NoticeMemory::default();
        assert_eq!(
            levels(
                &mut m,
                &[2900, 2507, 2506, 2500, 2507, 2506, 2450, 2404, 2400, 2420, 2380]
            ),
            [
                None,
                None,
                Some(L),
                None,
                None,
                None,
                None,
                Some(C),
                None,
                None,
                None
            ],
            "2506 is Low, 2404 Critical (registry 0x5A/0x60); noise around them is silent"
        );
        assert_eq!(levels(&mut m, &[2460, 2400]), [None, Some(C)]);
        assert_eq!(
            levels(&mut m, &[2556, 2500, 2400]),
            [None, Some(L), Some(C)]
        );
    }

    #[test]
    fn a_first_reading_already_critical_gives_one_critical_reminder() {
        let mut m = NoticeMemory::default();
        let (r, changed) = m.battery(MAC, 2300, &T);
        assert_eq!(
            r,
            Some(BatteryReminder {
                level: ReminderLevel::Critical,
                mv: 2300,
                threshold_mv: 2404
            })
        );
        assert!(changed);
        assert_eq!(
            m.battery(MAC, 2450, &T),
            (None, false),
            "Low after Critical: nothing"
        );
    }

    #[test]
    fn new_batteries_rearm_and_the_memory_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("akm-notices-{}", std::process::id()));
        let path = dir.join("notices.json");
        let mut m = NoticeMemory::default();
        assert!(m.battery(MAC, 2500, &T).0.is_some());
        assert!(m.firmware(MAC, Some("0x0050"), false).0);
        m.save(&path).unwrap();
        let mut back = NoticeMemory::load(&path);
        assert_eq!(back, m);
        assert_eq!(back.battery(MAC, 2500, &T).0, None, "restart: not repeated");
        assert!(
            !back.firmware(MAC, Some("0x0050"), false).0,
            "restart: not repeated"
        );
        assert!(back.rearm_battery(MAC));
        assert!(
            back.battery(MAC, 2500, &T).0.is_some(),
            "new set: armed again"
        );
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(NoticeMemory::load(&path), NoticeMemory::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn firmware_notice_once_per_version_and_rearmed_when_up_to_date() {
        let mut m = NoticeMemory::default();
        assert_eq!(m.firmware(MAC, Some("0x0050"), false), (true, true));
        assert_eq!(m.firmware(MAC, Some("0x0050"), false), (false, false));
        assert_eq!(
            m.firmware(MAC, None, false),
            (false, false),
            "unknown: keep"
        );
        assert_eq!(
            m.firmware(MAC, Some("0x0051"), false),
            (true, true),
            "newer: again"
        );
        assert_eq!(
            m.firmware(MAC, None, true),
            (false, true),
            "up to date: re-armed"
        );
        assert_eq!(m.firmware(MAC, Some("0x0051"), false), (true, true));
    }
}
