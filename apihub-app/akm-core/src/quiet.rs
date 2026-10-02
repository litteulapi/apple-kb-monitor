//! "Do not disturb" hours of the daemon's notifications (#91):
//! `[notifications] quiet_hours = "22:00-07:00"`.
//!
//! Pure: a set of ranges of the day in minutes, against a minute of the day
//! given by the caller (local time, [`local_minute_of_day`]; tests give their
//! own). A range whose end is before its start crosses midnight. Several
//! ranges are separated by commas (`"22:00-07:00, 12:30-13:30"`).

/// Minutes in a day.
pub const DAY_MIN: u16 = 24 * 60;

/// Ranges of the day during which non-critical notifications are held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuietHours {
    /// `(start, end)` in minutes of the day, start included, end excluded.
    ranges: Vec<(u16, u16)>,
}

fn parse_hm(s: &str) -> Option<u16> {
    let (h, m) = s.trim().split_once(':')?;
    let ok = |x: &str| !x.is_empty() && x.len() <= 2 && x.bytes().all(|b| b.is_ascii_digit());
    if !ok(h) || !ok(m) {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    // "24:00" is accepted as the end of the day.
    ((h < 24 && m < 60) || (h == 24 && m == 0)).then_some(h * 60 + m)
}

fn hm(min: u16) -> String {
    format!("{:02}:{:02}", min / 60 % 24, min % 60)
}

impl QuietHours {
    /// No quiet hours: nothing is ever held.
    pub fn none() -> Self {
        Self::default()
    }

    /// `"HH:MM-HH:MM"`, several separated by commas; `""` = none. A range of
    /// zero length (`"08:00-08:00"`) is refused: it would mean either never
    /// or always.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut ranges = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (a, b) = part
                .split_once('-')
                .ok_or_else(|| format!("{part:?} is not HH:MM-HH:MM"))?;
            let start = parse_hm(a).ok_or_else(|| format!("{a:?} is not a time HH:MM"))?;
            let end = parse_hm(b).ok_or_else(|| format!("{b:?} is not a time HH:MM"))?;
            let (start, end) = (start % DAY_MIN, end % DAY_MIN);
            if start == end {
                return Err(format!("{part:?} is an empty range"));
            }
            ranges.push((start, end));
        }
        Ok(Self { ranges })
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    fn in_range((start, end): (u16, u16), minute: u16) -> bool {
        if start < end {
            (start..end).contains(&minute)
        } else {
            minute >= start || minute < end
        }
    }

    /// Is `minute` (of the day, 0..1440) inside a quiet range?
    pub fn contains(&self, minute: u16) -> bool {
        let minute = minute % DAY_MIN;
        self.ranges.iter().any(|r| Self::in_range(*r, minute))
    }

    /// Minutes from `minute` to the end of the quiet period it is in (ranges
    /// that touch or overlap are followed through), `None` outside any
    /// range. Never more than a day.
    pub fn minutes_until_end(&self, minute: u16) -> Option<u16> {
        let mut at = minute % DAY_MIN;
        let mut total = 0u16;
        while total < DAY_MIN {
            let Some(end) = self
                .ranges
                .iter()
                .filter(|r| Self::in_range(**r, at))
                .map(|(_, end)| (*end + DAY_MIN - at) % DAY_MIN)
                .max()
            else {
                break;
            };
            total = total.saturating_add(end.max(1));
            at = (at + end) % DAY_MIN;
        }
        (total > 0).then_some(total.min(DAY_MIN))
    }

    /// `"22:00-07:00, 12:30-13:30"` (empty when there is none).
    pub fn describe(&self) -> String {
        self.ranges
            .iter()
            .map(|(a, b)| format!("{}-{}", hm(*a), hm(*b)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Minute of the day of unix time `ts` in the local time zone (the one of the
/// process: `TZ`, else `/etc/localtime`); UTC if the conversion fails.
pub fn local_minute_of_day(ts: u64) -> u16 {
    let t = ts as libc::time_t;
    // SAFETY: `tm` is plain data fully written by localtime_r, which is
    // given valid pointers and is the thread-safe variant.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return (ts % 86_400 / 60) as u16;
        }
        tm
    };
    (tm.tm_hour.clamp(0, 23) * 60 + tm.tm_min.clamp(0, 59)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u16, m: u16) -> u16 {
        h * 60 + m
    }

    #[test]
    fn a_range_across_midnight() {
        let q = QuietHours::parse("22:00-07:00").unwrap();
        assert!(!q.is_empty());
        for (minute, quiet) in [
            (at(21, 59), false),
            (at(22, 0), true),
            (at(23, 59), true),
            (at(0, 0), true),
            (at(6, 59), true),
            (at(7, 0), false),
            (at(12, 0), false),
        ] {
            assert_eq!(q.contains(minute), quiet, "{}", hm(minute));
        }
        assert_eq!(q.minutes_until_end(at(22, 0)), Some(9 * 60));
        assert_eq!(q.minutes_until_end(at(6, 59)), Some(1));
        assert_eq!(q.minutes_until_end(at(7, 0)), None);
        assert_eq!(q.describe(), "22:00-07:00");
    }

    #[test]
    fn a_range_inside_the_day_and_several_ranges() {
        let q = QuietHours::parse(" 12:30-13:30 , 22:00-24:00,00:00-07:00 ").unwrap();
        assert!(q.contains(at(12, 30)) && q.contains(at(13, 29)) && !q.contains(at(13, 30)));
        assert_eq!(q.minutes_until_end(at(13, 0)), Some(30));
        // 22:00-24:00 then 00:00-07:00 touch: one quiet period of 9 h.
        assert_eq!(q.minutes_until_end(at(22, 0)), Some(9 * 60));
        assert_eq!(q.minutes_until_end(at(23, 30)), Some(7 * 60 + 30));
        assert_eq!(q.describe(), "12:30-13:30, 22:00-00:00, 00:00-07:00");
    }

    #[test]
    fn nothing_is_quiet_without_a_range() {
        for q in [
            QuietHours::none(),
            QuietHours::parse("").unwrap(),
            QuietHours::parse(" , ").unwrap(),
        ] {
            assert!(q.is_empty());
            assert!((0..DAY_MIN).all(|m| !q.contains(m)));
            assert_eq!(q.minutes_until_end(0), None);
            assert_eq!(q.describe(), "");
        }
    }

    #[test]
    fn ranges_covering_the_whole_day_end_within_a_day() {
        let q = QuietHours::parse("00:00-12:00,12:00-24:00").unwrap();
        assert!((0..DAY_MIN).all(|m| q.contains(m)));
        assert_eq!(q.minutes_until_end(at(3, 0)), Some(DAY_MIN));
    }

    #[test]
    fn malformed_values_are_refused() {
        for bad in [
            "22:00",
            "22-7",
            "25:00-07:00",
            "22:60-07:00",
            "22:00-07:00-08:00x",
            "aa:bb-cc:dd",
            "08:00-08:00",
            "22:00 07:00",
            "-",
            "022:00-07:00",
        ] {
            assert!(QuietHours::parse(bad).is_err(), "{bad:?}");
        }
        assert!(
            QuietHours::parse("9:5-10:0").is_ok(),
            "short digits are fine"
        );
    }

    #[test]
    fn the_local_minute_follows_the_time_zone() {
        // 2026-10-02 12:34:56 UTC.
        let ts = 1_790_944_496;
        let m = local_minute_of_day(ts);
        assert!(m < DAY_MIN);
        // Whatever the zone, minutes past the hour agree for whole-hour
        // offsets and half-hour ones differ by 30: check the invariant that
        // one minute later is one minute later.
        assert_eq!((local_minute_of_day(ts + 60) + DAY_MIN - m) % DAY_MIN, 1);
    }
}
