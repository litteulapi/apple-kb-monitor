//! When a notification is shown: the quiet hours (#91) and "Remind me
//! tomorrow" (#110), in front of [`crate::notify::deliver`].
//!
//! * **Quiet hours** (`[notifications] quiet_hours = "22:00-07:00"`): inside
//!   the range a notification that is not critical is not sent; it is
//!   journalled ("held until 07:00") and kept, one per replacement slot, to be
//!   shown when the range ends. A **critical** notification (critical
//!   batteries, re-pairing needed, an error the user must see) always goes
//!   out at once.
//! * **Remind me tomorrow**: the button puts the notification aside for
//!   24 hours ([`akm_core::deferred::REMIND_AFTER_S`]).
//!
//! Both live in one persistent, bounded store
//! ([`akm_core::deferred::DeferredStore`]), so a daemon restart loses nothing.
//! The clock is given by the caller ([`Now`]); the process-wide entry points
//! ([`admit`], [`take_due`], [`remind`]) use the wall clock, replaceable in
//! tests ([`set_clock`]).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use akm_core::alerts::Urgency;
use akm_core::deferred::{DeferredStore, Reason};
use akm_core::history::Clock;
use akm_core::quiet::QuietHours;

use crate::notify::Notification;

/// The instant a decision is taken at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    /// Unix seconds.
    pub unix: u64,
    /// Local minute of the day (0..1440).
    pub minute: u16,
}

impl Now {
    /// The wall clock, local time zone.
    pub fn system() -> Self {
        let unix = akm_core::history::SystemClock.now();
        Self {
            unix,
            minute: akm_core::quiet::local_minute_of_day(unix),
        }
    }
}

fn hm(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

/// Quiet hours and deferred notifications of one daemon.
#[derive(Debug, Default)]
pub struct Policy {
    quiet: QuietHours,
    store: DeferredStore,
    /// Where the store is kept (`None`: memory only, tests and `--no-history`).
    path: Option<PathBuf>,
}

impl Policy {
    pub fn new(quiet: QuietHours, path: Option<PathBuf>) -> Self {
        let store = path.as_deref().map(DeferredStore::load).unwrap_or_default();
        Self { quiet, store, path }
    }

    pub fn quiet_hours(&self) -> &QuietHours {
        &self.quiet
    }

    /// Notifications waiting (held or to be reminded).
    pub fn waiting(&self) -> usize {
        self.store.len()
    }

    fn save(&self) {
        if let Some(p) = self.path.as_deref() {
            if let Err(e) = self.store.save(p) {
                tracing::warn!("cannot save {}: {e}", p.display());
            }
        }
    }

    /// May `n` be shown now? `Some(n)`: send it. `None`: it is held by the
    /// quiet hours (journalled, kept until they end). Critical always passes.
    pub fn admit(&mut self, n: Notification, now: Now) -> Option<Notification> {
        if n.urgency == Urgency::Critical {
            return Some(n);
        }
        let Some(left) = self.quiet.minutes_until_end(now.minute) else {
            return Some(n);
        };
        // The range ends `left` minutes after the start of this minute.
        let due = now.unix - now.unix % 60 + u64::from(left) * 60;
        tracing::info!(
            "notification held until {} (quiet hours {}): [{}] {}",
            hm((now.minute + left) % akm_core::quiet::DAY_MIN),
            self.quiet.describe(),
            n.event.id(),
            n.summary
        );
        self.store
            .defer(Reason::Quiet, n.event.slot(), n.to_stored(), due);
        self.save();
        None
    }

    /// "Remind me tomorrow" pressed on `n` at `now`. Returns when it is due.
    pub fn remind(&mut self, n: &Notification, now: Now) -> u64 {
        let due = self
            .store
            .remind_tomorrow(n.event.slot(), n.to_stored(), now.unix);
        tracing::info!(
            "notification [{}] to be shown again in 24 h: {}",
            n.event.id(),
            n.summary
        );
        self.save();
        due
    }

    /// What must be shown again at `now` (end of the quiet hours, reminder
    /// due). The caller hands each one to [`Policy::admit`] again: a reminder
    /// due inside the quiet hours waits for their end like any other.
    pub fn take_due(&mut self, now: Now) -> Vec<Notification> {
        if self.store.next_due().is_none_or(|t| t > now.unix) {
            return Vec::new();
        }
        let due = self.store.take_due(now.unix);
        self.save();
        due.into_iter()
            .filter_map(|d| {
                let n = Notification::from_stored(&d.notification);
                if n.is_none() {
                    tracing::warn!(
                        "deferred notification of unknown event {:?} dropped",
                        d.notification.event
                    );
                }
                match d.reason {
                    Reason::Quiet => tracing::info!(
                        "quiet hours over: showing [{}] {}",
                        d.notification.event,
                        d.notification.summary
                    ),
                    Reason::Remind => tracing::info!(
                        "reminder due: showing [{}] {}",
                        d.notification.event,
                        d.notification.summary
                    ),
                }
                n
            })
            .collect()
    }

    /// Forget what waits in `slot` (new batteries: no battery reminder).
    pub fn cancel_slot(&mut self, slot: &str) -> usize {
        let n = self.store.cancel_slot(slot);
        if n > 0 {
            self.save();
        }
        n
    }
}

// ── Process-wide policy ────────────────────────────────────────────────────

fn policy() -> &'static Mutex<Policy> {
    static P: OnceLock<Mutex<Policy>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Policy::default()))
}

type ClockFn = Arc<dyn Fn() -> Now + Send + Sync>;

fn clock_slot() -> &'static Mutex<Option<ClockFn>> {
    static C: OnceLock<Mutex<Option<ClockFn>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The time the process-wide policy decides at.
pub fn now() -> Now {
    let custom = lock(clock_slot()).clone();
    custom.map_or_else(Now::system, |f| f())
}

/// Replace the clock of the process-wide policy (tests).
pub fn set_clock(f: impl Fn() -> Now + Send + Sync + 'static) {
    *lock(clock_slot()) = Some(Arc::new(f));
}

/// Set the quiet hours and where deferred notifications are kept (done once
/// by `main` from `config.toml`; before that nothing is held nor persisted).
pub fn configure(quiet: QuietHours, path: Option<PathBuf>) {
    if !quiet.is_empty() {
        tracing::info!(
            "notifications: quiet hours {} (critical ones always shown)",
            quiet.describe()
        );
    }
    *lock(policy()) = Policy::new(quiet, path);
}

/// [`Policy::admit`] of the process-wide policy, at [`now`].
pub fn admit(n: Notification) -> Option<Notification> {
    lock(policy()).admit(n, now())
}

/// [`Policy::remind`] of the process-wide policy, at [`now`].
pub fn remind(n: &Notification) -> u64 {
    lock(policy()).remind(n, now())
}

/// [`Policy::take_due`] of the process-wide policy, at [`now`].
pub fn take_due() -> Vec<Notification> {
    lock(policy()).take_due(now())
}

/// [`Policy::cancel_slot`] of the process-wide policy.
pub fn cancel_slot(slot: &str) -> usize {
    lock(policy()).cancel_slot(slot)
}

/// Notifications waiting in the process-wide policy.
pub fn waiting() -> usize {
    lock(policy()).waiting()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::{self, Event, Lang};
    use akm_core::alerts::Crossing;
    use akm_core::chemistry::AlertBasis;
    use akm_core::link::LinkEvent;
    use akm_core::reminder::{BatteryReminder, ReminderLevel};

    /// 2026-10-02 00:00:00 UTC; the tests pretend local time is UTC.
    const DAY: u64 = 1_790_899_200;

    fn at(day: u64, h: u64, m: u64) -> Now {
        Now {
            unix: DAY + day * 86_400 + h * 3600 + m * 60 + 17,
            minute: (h * 60 + m) as u16,
        }
    }

    fn low() -> Notification {
        let c = Crossing {
            threshold: 30,
            pct: 29.0,
            urgency: Urgency::Normal,
        };
        notify::crossing_notification(&c, AlertBasis::Estimate, Lang::Fr)
    }

    fn critical() -> Notification {
        let c = Crossing {
            threshold: 5,
            pct: 4.0,
            urgency: Urgency::Critical,
        };
        notify::crossing_notification(&c, AlertBasis::Estimate, Lang::Fr)
    }

    fn link(up: bool) -> Notification {
        let mac = "AA:BB:CC:DD:EE:F1".to_string();
        let ev = if up {
            LinkEvent::Reconnected {
                mac,
                pct: Some(80.0),
            }
        } else {
            LinkEvent::Disconnected { mac }
        };
        notify::link_notification(&ev, Lang::Fr)
    }

    fn quiet() -> Policy {
        Policy::new(QuietHours::parse("22:00-07:00").unwrap(), None)
    }

    #[test]
    fn outside_the_quiet_hours_everything_is_sent() {
        let mut p = quiet();
        assert_eq!(p.admit(low(), at(0, 21, 59)), Some(low()));
        assert_eq!(p.admit(link(false), at(0, 7, 0)), Some(link(false)));
        assert_eq!(p.waiting(), 0);
        // No quiet hours configured: never held.
        let mut none = Policy::default();
        assert_eq!(none.admit(low(), at(0, 23, 0)), Some(low()));
    }

    #[test]
    fn a_non_critical_notification_is_held_and_shown_when_the_range_ends() {
        let mut p = quiet();
        assert_eq!(p.admit(low(), at(0, 23, 10)), None, "held");
        assert_eq!(p.waiting(), 1);
        // Still inside the range, also after midnight: nothing comes out.
        for t in [at(0, 23, 59), at(1, 0, 0), at(1, 3, 30), at(1, 6, 59)] {
            assert!(p.take_due(t).is_empty(), "{t:?}");
        }
        // 07:00: it comes out, once, and is admitted.
        let due = p.take_due(at(1, 7, 0));
        assert_eq!(due, [low()]);
        assert_eq!(p.admit(due[0].clone(), at(1, 7, 0)), Some(low()));
        assert!(p.take_due(at(1, 7, 1)).is_empty());
        assert_eq!(p.waiting(), 0);
    }

    #[test]
    fn the_critical_one_always_goes_out_at_once() {
        let mut p = quiet();
        for t in [at(0, 22, 0), at(1, 2, 0), at(1, 6, 59)] {
            assert_eq!(p.admit(critical(), t), Some(critical()), "{t:?}");
        }
        // Critical reminder of the keyboard's own threshold, re-pairing notice.
        let rem = notify::reminder_notification(
            &BatteryReminder {
                level: ReminderLevel::Critical,
                mv: 2400,
                threshold_mv: 2404,
            },
            Some(4.0),
            Lang::Fr,
        );
        assert!(p.admit(rem, at(1, 2, 0)).is_some());
        let repair = notify::notice_notification(Event::RepairNeeded, "s", "b", Urgency::Critical);
        assert!(p.admit(repair, at(1, 2, 0)).is_some());
        assert_eq!(p.waiting(), 0, "nothing critical is ever kept back");
    }

    #[test]
    fn one_notification_per_slot_is_kept_the_latest() {
        let mut p = quiet();
        assert!(p.admit(link(false), at(0, 22, 30)).is_none());
        assert!(p.admit(link(true), at(0, 23, 30)).is_none());
        assert!(p.admit(low(), at(1, 1, 0)).is_none());
        assert_eq!(
            p.waiting(),
            2,
            "link slot: the reconnection replaced the disconnection"
        );
        let due = p.take_due(at(1, 7, 0));
        assert_eq!(due.len(), 2);
        assert!(due.contains(&link(true)) && due.contains(&low()));
    }

    #[test]
    fn remind_me_tomorrow_comes_back_24_hours_later() {
        let mut p = Policy::default();
        let t0 = at(0, 10, 0);
        assert_eq!(p.remind(&low(), t0), t0.unix + 86_400);
        assert!(p.take_due(at(0, 23, 0)).is_empty());
        let almost = Now {
            unix: t0.unix + 86_399,
            minute: 9 * 60 + 59,
        };
        assert!(p.take_due(almost).is_empty(), "one second early");
        let due = p.take_due(Now {
            unix: t0.unix + 86_400,
            minute: 600,
        });
        assert_eq!(due, [low()], "same text, same buttons");
        assert_eq!(p.waiting(), 0);
    }

    #[test]
    fn a_reminder_due_inside_the_quiet_hours_waits_for_their_end() {
        let mut p = quiet();
        let t0 = at(0, 23, 30);
        // Pressed at 23:30 on a notification shown before the range began.
        p.remind(&low(), t0);
        let next = Now {
            unix: t0.unix + 86_400,
            minute: 23 * 60 + 30,
        };
        let due = p.take_due(next);
        assert_eq!(due.len(), 1);
        assert!(p.admit(due[0].clone(), next).is_none(), "held again: quiet");
        assert!(p.take_due(at(2, 6, 0)).is_empty());
        assert_eq!(p.take_due(at(2, 7, 0)), [low()]);
    }

    #[test]
    fn new_batteries_cancel_a_pending_battery_reminder() {
        let mut p = quiet();
        p.remind(&low(), at(0, 10, 0));
        assert!(p.admit(link(false), at(0, 23, 0)).is_none());
        assert_eq!(p.cancel_slot("battery"), 1);
        assert_eq!(p.waiting(), 1, "the link notification still waits");
        assert_eq!(p.take_due(at(3, 12, 0)), [link(false)]);
    }

    #[test]
    fn what_waits_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("akm-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("deferred-notifications.json");
        let q = || QuietHours::parse("22:00-07:00").unwrap();
        let mut p = Policy::new(q(), Some(path.clone()));
        p.remind(&low(), at(0, 10, 0));
        assert!(p.admit(link(false), at(0, 23, 0)).is_none());
        drop(p);
        let mut back = Policy::new(q(), Some(path.clone()));
        assert_eq!(back.waiting(), 2);
        assert_eq!(back.take_due(at(1, 7, 0)), [link(false)]);
        assert_eq!(back.take_due(at(1, 10, 1)), [low()]);
        // Emptied on disk too.
        assert_eq!(Policy::new(q(), Some(path)).waiting(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
