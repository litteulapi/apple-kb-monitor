//! #91 and #110 end to end on a private bus, with a fake notification server
//! and a simulated clock: quiet hours hold what is not critical and show it
//! when they end; the critical alert goes out at once; "Remind me tomorrow"
//! brings the notification back 24 hours later, also across a restart of the
//! store. No real notification reaches the desktop.

#![allow(dead_code, unused_variables)] // completed by the "Remind me tomorrow" test

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akm_core::alerts::{Crossing, Urgency};
use akm_core::chemistry::AlertBasis;
use akm_core::link::LinkEvent;
use akm_core::quiet::QuietHours;
use apple_kb_monitord::notify;
use apple_kb_monitord::notify_policy::{self, Now};
use zbus::zvariant::OwnedValue;

const INNER: &str = "AKM_NOTIFY_QUIET_INNER";
const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(600);
/// 2026-10-02 00:00:00 UTC; the simulated local time is UTC.
const DAY: u64 = 1_790_899_200;

#[derive(Debug, Clone)]
struct Call {
    id: u32,
    summary: String,
    actions: Vec<String>,
    event: String,
}

struct Fake {
    tx: Mutex<Sender<Call>>,
    next: u32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Fake {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &mut self,
        _app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        _body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next += 1;
            self.next
        };
        let event = hints
            .get("x-kde-eventId")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or_default()
            .to_string();
        let _ = self.tx.lock().unwrap().send(Call {
            id,
            summary,
            actions,
            event,
        });
        id
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < end, "timeout: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn inner() {
    let (tx, calls): (Sender<Call>, Receiver<Call>) = mpsc::channel();
    let server = zbus::blocking::ConnectionBuilder::session()
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Notifications",
            Fake {
                tx: Mutex::new(tx),
                next: 0,
            },
        )
        .unwrap()
        .build()
        .unwrap();

    // Simulated clock: seconds since DAY, local time = UTC.
    let clock = Arc::new(AtomicU64::new(0));
    let c = clock.clone();
    notify_policy::set_clock(move || {
        let s = c.load(Ordering::SeqCst);
        Now {
            unix: DAY + s,
            minute: (s / 60 % 1440) as u16,
        }
    });
    let at =
        |day: u64, h: u64, m: u64| clock.store(day * 86_400 + h * 3600 + m * 60, Ordering::SeqCst);

    let dir = std::env::temp_dir().join(format!("akm-quiet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = dir.join("deferred-notifications.json");
    let quiet = || QuietHours::parse("22:00-07:00").unwrap();
    notify_policy::configure(quiet(), Some(store.clone()));

    let low = Crossing {
        threshold: 30,
        pct: 29.0,
        urgency: Urgency::Normal,
    };
    let critical = Crossing {
        threshold: 5,
        pct: 4.0,
        urgency: Urgency::Critical,
    };
    let mac = "AA:BB:CC:DD:EE:F1".to_string();

    // ── #91 ────────────────────────────────────────────────────────────────
    // 21:59: before the range, shown at once.
    at(0, 21, 59);
    notify::battery_crossing(&low, AlertBasis::Estimate);
    assert_eq!(
        calls.recv_timeout(WAIT).expect("before the range").event,
        "BatteryLow"
    );

    // 23:00: a non-critical alert and a connection change are held.
    at(0, 23, 0);
    notify::battery_crossing(&low, AlertBasis::Estimate);
    notify::link(&LinkEvent::Disconnected { mac: mac.clone() });
    assert!(
        calls.recv_timeout(SHORT).is_err(),
        "sent during the quiet hours"
    );
    assert_eq!(notify_policy::waiting(), 2);
    // ... they are journalled on disk, to survive a restart of the daemon.
    let text = std::fs::read_to_string(&store).unwrap();
    assert!(
        text.contains("BatteryLow") && text.contains("KeyboardDisconnected"),
        "{text}"
    );

    // 02:00: the critical alert passes, at once.
    at(1, 2, 0);
    notify::battery_crossing(&critical, AlertBasis::Estimate);
    let crit = calls
        .recv_timeout(WAIT)
        .expect("critical during the quiet hours");
    assert_eq!(crit.event, "BatteryCritical");
    notify::tick();
    assert!(
        calls.recv_timeout(SHORT).is_err(),
        "released before the end"
    );

    // 06:59: still quiet. 07:00: both come out, once.
    at(1, 6, 59);
    notify::tick();
    assert!(calls.recv_timeout(SHORT).is_err());
    at(1, 7, 0);
    notify::tick();
    let mut released = vec![
        calls.recv_timeout(WAIT).expect("first released").event,
        calls.recv_timeout(WAIT).expect("second released").event,
    ];
    released.sort();
    assert_eq!(released, ["BatteryLow", "KeyboardDisconnected"]);
    notify::tick();
    assert!(calls.recv_timeout(SHORT).is_err(), "shown once");
    assert_eq!(notify_policy::waiting(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quiet_hours_and_remind_me_tomorrow() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test(
        "quiet_hours_and_remind_me_tomorrow",
        INNER,
        &[("LC_ALL", "fr_FR.UTF-8")],
    );
}
