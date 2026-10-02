//! Battery health advice (#108): "batteries changed too often".
//!
//! Two sets of batteries in a row that each lasted less than
//! [`SHORT_SET_S`] (30 days) are not normal for this keyboard (months on
//! alkaline cells): the advice is raised once the second one is replaced, and
//! stays until the set in use has itself lasted 30 days.
//!
//! Pure: computed from the battery sets of the history
//! ([`crate::batteries::battery_sets`]). A set whose beginning is not known
//! (the history starts with it) is never counted: its real life is longer
//! than what was recorded.

use serde::{Deserialize, Serialize};

use crate::batteries::BatterySet;

/// A set that lasted less than this is "short".
pub const SHORT_SET_S: u64 = 30 * 86_400;
/// Consecutive short sets needed for the advice.
pub const SHORT_SETS: usize = 2;

/// The last two sets of batteries were both replaced within 30 days.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortLife {
    /// Life in days of the two sets, older first.
    pub days: [f64; SHORT_SETS],
    /// Unix time the second short set was replaced.
    pub since: u64,
}

/// The advice for a history split into `sets` (oldest first), if it applies.
pub fn short_life(sets: &[BatterySet]) -> Option<ShortLife> {
    let ended: Vec<&BatterySet> = sets.iter().filter(|s| s.ended).collect();
    if ended.len() < SHORT_SETS {
        return None;
    }
    let last = &ended[ended.len() - SHORT_SETS..];
    if !last
        .iter()
        .all(|s| s.start_known && s.duration_s < SHORT_SET_S)
    {
        return None;
    }
    // The set in use already lasted a normal time: the trouble is over.
    if sets
        .last()
        .is_some_and(|cur| !cur.ended && cur.duration_s >= SHORT_SET_S)
    {
        return None;
    }
    let day = |s: &BatterySet| (s.duration_s as f64 / 86_400.0 * 10.0).round() / 10.0;
    let newest = last[SHORT_SETS - 1];
    Some(ShortLife {
        days: [day(last[0]), day(newest)],
        since: newest.start_ts + newest.duration_s,
    })
}

/// One line for a menu or a tooltip.
pub fn line(a: &ShortLife, french: bool) -> String {
    if french {
        format!(
            "Piles chang\u{e9}es trop souvent : {:.0} j puis {:.0} j (moins de 30 j)",
            a.days[0], a.days[1]
        )
    } else {
        format!(
            "Batteries replaced too often: {:.0} d then {:.0} d (under 30 d)",
            a.days[0], a.days[1]
        )
    }
}

/// What to do about it, for a notification body.
pub fn recommendation(french: bool) -> &'static str {
    if french {
        "V\u{e9}rifiez la chimie d\u{e9}clar\u{e9}e ([battery] chemistry), essayez des piles neuves \
         d'une autre s\u{e9}rie ou des accus NiMH \u{e0} faible autod\u{e9}charge, et \u{e9}teignez le clavier \
         quand il ne sert pas."
    } else {
        "Check the declared chemistry ([battery] chemistry), try fresh cells of another batch or \
         low self-discharge NiMH cells, and switch the keyboard off when it is not used."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batteries::battery_sets;
    use crate::history::HistoryEntry;

    const T0: u64 = 1_780_000_000;
    const DAY: u64 = 86_400;

    /// A history made of sets lasting `days` each: 100 % down to 10 %, a
    /// sample every 6 hours, the next set starting where the previous ended.
    fn history(days: &[u64], current_days: u64) -> Vec<HistoryEntry> {
        let mut out = Vec::new();
        let mut t = T0;
        let mut set = |len: u64, t: &mut u64| {
            let steps = len * 4;
            for i in 0..steps {
                let pct = 100.0 - 90.0 * i as f64 / steps as f64;
                out.push(HistoryEntry::sample(*t + i * DAY / 4, pct, None));
            }
            *t += len * DAY;
        };
        // The first set: its beginning is unknown (the history starts there).
        set(40, &mut t);
        for d in days {
            set(*d, &mut t);
        }
        if current_days > 0 {
            set(current_days, &mut t);
        }
        out
    }

    #[test]
    fn two_short_sets_in_a_row_raise_the_advice() {
        // Unknown first set, then 12 days, then 18 days, then fresh cells.
        let sets = battery_sets(&history(&[12, 18], 1));
        assert_eq!(sets.len(), 4);
        let a = short_life(&sets).expect("advice");
        assert_eq!(a.days, [12.0, 18.0]);
        assert_eq!(a.since, T0 + (40 + 12 + 18) * DAY);
        assert!(line(&a, true).contains("12 j puis 18 j"));
        assert!(line(&a, false).contains("12 d then 18 d"));
        assert!(recommendation(true).contains("NiMH") && recommendation(false).contains("NiMH"));
    }

    #[test]
    fn one_short_set_or_a_normal_one_in_between_raises_nothing() {
        assert_eq!(short_life(&battery_sets(&history(&[12], 1))), None);
        assert_eq!(short_life(&battery_sets(&history(&[12, 45], 1))), None);
        assert_eq!(short_life(&battery_sets(&history(&[45, 12], 1))), None);
        assert_eq!(short_life(&battery_sets(&history(&[], 20))), None);
        assert_eq!(short_life(&[]), None);
        // Exactly 30 days is not "less than 30 days".
        assert_eq!(short_life(&battery_sets(&history(&[30, 30], 1))), None);
        assert!(short_life(&battery_sets(&history(&[29, 29], 1))).is_some());
    }

    #[test]
    fn a_set_whose_beginning_is_unknown_is_never_counted() {
        // The history starts with a set seen for 10 days only, then one of
        // 12 days: only ONE short set is known for sure.
        let mut h = Vec::new();
        for i in 0..40u64 {
            h.push(HistoryEntry::sample(
                T0 + i * DAY / 4,
                50.0 - i as f64,
                None,
            ));
        }
        for i in 0..48u64 {
            h.push(HistoryEntry::sample(
                T0 + 10 * DAY + i * DAY / 4,
                100.0 - i as f64,
                None,
            ));
        }
        h.push(HistoryEntry::sample(T0 + 22 * DAY, 100.0, None));
        let sets = battery_sets(&h);
        assert_eq!(sets.len(), 3);
        assert!(!sets[0].start_known && sets[0].ended);
        assert_eq!(short_life(&sets), None);
    }

    #[test]
    fn the_advice_ends_once_the_set_in_use_lasted_a_normal_time() {
        assert!(short_life(&battery_sets(&history(&[12, 18], 29))).is_some());
        assert_eq!(short_life(&battery_sets(&history(&[12, 18], 31))), None);
        // The second short set was just replaced (one sample on the new one).
        let mut h = history(&[12, 18], 0);
        h.push(HistoryEntry::sample(T0 + 70 * DAY, 100.0, None));
        assert!(short_life(&battery_sets(&h)).is_some());
    }
}
