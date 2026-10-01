//! Desktop notifications through `org.freedesktop.Notifications`, in zbus
//! directly (no `notify-rust`: one zbus version in the tree).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use akm_core::alerts::{Crossing, Urgency};
use akm_core::batteries::Replacement;
use akm_core::link::{self, LinkEvent};
use zbus::zvariant::Value;

/// Longest wait for one `Notify` call before it is abandoned.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Notifications waiting for the sender thread; beyond that they are dropped.
const QUEUE: usize = 16;
/// Calls abandoned and still blocked before new ones are refused.
const MAX_BLOCKED: usize = 4;

type Job = Box<dyn FnOnce() + Send>;

/// Hands notifications to a dedicated thread (#162). The caller (the
/// acquisition actor) only does a `try_send`: a silent notification server can
/// delay nothing but the notifications themselves.
pub struct Notifier {
    tx: SyncSender<Job>,
}

impl Notifier {
    pub fn new(call_timeout: Duration, max_blocked: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Job>(QUEUE);
        let spawned = thread::Builder::new()
            .name("kb-notify".into())
            .spawn(move || {
                let in_flight = Arc::new(AtomicUsize::new(0));
                for job in rx {
                    if in_flight.load(Ordering::SeqCst) >= max_blocked {
                        tracing::warn!("notification dropped: the notification server is stuck");
                        continue;
                    }
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    let (done_tx, done_rx) = mpsc::channel::<()>();
                    let counter = in_flight.clone();
                    let started = thread::Builder::new().name("kb-notify-call".into()).spawn(move || {
                        job();
                        counter.fetch_sub(1, Ordering::SeqCst);
                        let _ = done_tx.send(());
                    });
                    match started {
                        Ok(_) => {
                            if done_rx.recv_timeout(call_timeout).is_err() {
                                tracing::warn!(
                                    "notification server did not answer within {call_timeout:?}"
                                );
                            }
                        }
                        Err(_) => {
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("cannot start the notification thread: {e}");
        }
        Self { tx }
    }

    /// Queue a job; never blocks. False if it was dropped.
    pub fn submit(&self, job: impl FnOnce() + Send + 'static) -> bool {
        match self.tx.try_send(Box::new(job)) {
            Ok(()) => true,
            Err(_) => {
                tracing::warn!("notification dropped (queue full or sender gone)");
                false
            }
        }
    }
}

fn notifier() -> &'static Notifier {
    static N: OnceLock<Notifier> = OnceLock::new();
    N.get_or_init(|| Notifier::new(CALL_TIMEOUT, MAX_BLOCKED))
}

/// Send a critical notification on the session bus; errors are logged,
/// never fatal (a headless session has no notification server). Returns at
/// once: the call runs on the notification thread.
pub fn send(summary: &str, body: &str, icon: &str) {
    send_with(summary, body, icon, Urgency::Critical, false);
}

/// Send with an urgency; `transient` notifications are not kept in the
/// history of the notification server (connection changes).
pub fn send_with(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) {
    let (summary, body, icon) = (summary.to_string(), body.to_string(), icon.to_string());
    notifier().submit(move || send_blocking(&summary, &body, &icon, urgency, transient));
}

/// The blocking D-Bus call (notification thread only).
pub fn send_blocking(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) {
    let res = (|| -> zbus::Result<u32> {
        let conn = zbus::blocking::Connection::session()?;
        let mut hints: HashMap<&str, Value<'_>> =
            HashMap::from([("urgency", Value::U8(urgency.hint()))]);
        if transient {
            hints.insert("transient", Value::Bool(true));
        }
        let reply = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "apple-kb-monitord",
                0u32,
                icon,
                summary,
                body,
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )?;
        reply.body().deserialize()
    })();
    if let Err(e) = res {
        tracing::warn!("notification not sent: {e}");
    }
}

/// Low-battery alert text.
pub fn low_battery_text(pct: f64) -> (String, String) {
    (
        "Apple Keyboard \u{2014} Low Battery".into(),
        format!("Battery at {:.0}% \u{2014} charge soon", pct),
    )
}

pub fn low_battery(pct: f64) {
    let (s, b) = low_battery_text(pct);
    send(&s, &b, "battery-caution");
}

/// Text and icon of a threshold alert (#82).
pub fn crossing_text(c: &Crossing) -> (String, String, &'static str) {
    let (summary, _) = low_battery_text(c.pct);
    let body = if c.urgency == Urgency::Critical {
        format!(
            "Battery at {:.0}% \u{2014} replace the batteries now",
            c.pct
        )
    } else {
        format!(
            "Battery at {:.0}% (below {}%) \u{2014} plan to replace the batteries",
            c.pct, c.threshold
        )
    };
    let icon = if c.urgency == Urgency::Critical {
        "battery-empty"
    } else {
        "battery-caution"
    };
    (summary, body, icon)
}

pub fn battery_crossing(c: &Crossing) {
    let (s, b, icon) = crossing_text(c);
    send_with(&s, &b, icon, c.urgency, false);
}

/// Disconnected / reconnected (#84): low urgency, transient.
pub fn link(ev: &LinkEvent) {
    let (s, b) = link::text(ev);
    let icon = match ev {
        LinkEvent::Disconnected { .. } => "input-keyboard-virtual-off",
        LinkEvent::Reconnected { .. } => "input-keyboard",
    };
    send_with(&s, &b, icon, Urgency::Low, true);
}

/// Text of the "new batteries" notification (#85).
pub fn replaced_text(r: &Replacement) -> (String, String) {
    let before = r.pct_before.map_or("?".to_string(), |p| format!("{p:.0}%"));
    (
        "Apple Keyboard \u{2014} new batteries".into(),
        format!(
            "Battery {before} \u{2192} {:.0}%; low-battery alerts re-armed",
            r.pct_after
        ),
    )
}

pub fn battery_replaced(r: &Replacement) {
    let (s, b) = replaced_text(r);
    send_with(&s, &b, "battery-full", Urgency::Normal, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_and_replacement_texts() {
        let c = Crossing {
            threshold: 30,
            pct: 29.6,
            urgency: Urgency::Normal,
        };
        let (_, b, icon) = crossing_text(&c);
        assert_eq!(
            b,
            "Battery at 30% (below 30%) \u{2014} plan to replace the batteries"
        );
        assert_eq!(icon, "battery-caution");
        let c = Crossing {
            threshold: 5,
            pct: 4.0,
            urgency: Urgency::Critical,
        };
        assert!(crossing_text(&c).1.contains("replace the batteries now"));
        let r = Replacement {
            ts: 0,
            pct_before: Some(3.0),
            pct_after: 100.0,
            voltage_before: None,
            voltage_after: None,
        };
        assert_eq!(
            replaced_text(&r).1,
            "Battery 3% \u{2192} 100%; low-battery alerts re-armed"
        );
    }

    #[test]
    fn mute_server_never_blocks_the_caller() {
        use std::time::Instant;
        // #162: jobs that never return must not delay submit(), nor the next ones.
        let n = Notifier::new(Duration::from_millis(100), 2);
        let t = Instant::now();
        for _ in 0..8 {
            n.submit(|| thread::sleep(Duration::from_secs(30)));
        }
        assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
        // Once the 2 allowed blocked calls are stuck, the rest is dropped,
        // never run, never waited for.
        let (tx, rx) = mpsc::channel();
        thread::sleep(Duration::from_millis(500));
        n.submit(move || {
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    fn healthy_server_gets_every_job_in_order() {
        let n = Notifier::new(Duration::from_secs(1), 4);
        let (tx, rx) = mpsc::channel();
        for i in 0..5 {
            let tx = tx.clone();
            n.submit(move || {
                let _ = tx.send(i);
            });
        }
        let got: Vec<i32> = (0..5)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn low_battery_text_rounds() {
        let (s, b) = super::low_battery_text(12.4);
        assert!(s.contains("Low Battery"));
        assert_eq!(b, "Battery at 12% \u{2014} charge soon");
    }
}
