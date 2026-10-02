//! #96 on a private bus: `History(since)` is bounded by the daemon and
//! `HistoryMax(since, max)` lets the client choose the bound, on the root
//! object and on the keyboard object. No keyboard, no real daemon.

use std::sync::Arc;

use akm_core::history::{History, HistoryEntry, HistoryEvent, SystemClock};
use akm_core::history_limits::DEFAULT_MAX_POINTS;
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::devices::{device_path, DEVICE_INTERFACE};
use apple_kb_monitord::service;
use zbus::blocking::Connection;

const INNER: &str = "AKM_HISTORY_BOUNDED_INNER";
const MAC: &str = "AA:BB:CC:DD:EE:F1";
const T0: u64 = 1_790_000_000;

fn history(
    c: &Connection,
    path: &str,
    iface: &str,
    member: &str,
    max: Option<u32>,
) -> Vec<HistoryEntry> {
    let reply = match max {
        Some(m) => c.call_method(Some(service::BUS_NAME), path, Some(iface), member, &(T0, m)),
        None => c.call_method(Some(service::BUS_NAME), path, Some(iface), member, &(T0,)),
    }
    .unwrap();
    let json: String = reply.body().deserialize().unwrap();
    serde_json::from_str(&json).unwrap()
}

fn inner() {
    let dir = std::env::temp_dir().join(format!("akm-histb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let h = History::new(dir.join("h.jsonl"), SystemClock);
    // 5000 samples, one battery replacement in the middle.
    for i in 0..5_000u64 {
        let mut e = HistoryEntry::measured(T0 + i * 300, 80.0, Some(2800), Some(2763));
        if i == 2_500 {
            e.event = Some(HistoryEvent::BatteryReplaced);
        }
        h.append_entry(&e).unwrap();
    }
    let watch = Arc::new(Watch::new());
    let mut k = KbReport::default();
    k.battery.percentage_fine = Some(80.0);
    k.device.mac = Some(MAC.into());
    watch.publish(Snapshot {
        connected: true,
        keyboard: Some(k),
        ..Default::default()
    });
    let _server = service::serve(watch, Mailbox::new(), Some(Arc::new(h))).expect("serve");
    let c = Connection::session().unwrap();
    let dev = device_path(MAC).unwrap();

    for (path, iface) in [
        (service::OBJECT_PATH, service::INTERFACE),
        (dev.as_str(), DEVICE_INTERFACE),
    ] {
        // Without a bound: the daemon's own (2000 points), never the 5000.
        let all = history(&c, path, iface, "History", None);
        assert_eq!(all.len(), DEFAULT_MAX_POINTS, "{iface}");
        assert_eq!(all[0].ts, T0);
        assert_eq!(all.last().unwrap().ts, T0 + 4_999 * 300);
        assert!(
            all.iter().any(|e| e.event.is_some()),
            "the replacement is kept"
        );
        // The client's bound.
        let few = history(&c, path, iface, "HistoryMax", Some(50));
        assert_eq!(few.len(), 50, "{iface}");
        assert!(few.iter().any(|e| e.event.is_some()));
        assert!(few.windows(2).all(|w| w[0].ts < w[1].ts));
        // 0 = the default bound; a huge bound gives everything there is.
        assert_eq!(
            history(&c, path, iface, "HistoryMax", Some(0)).len(),
            DEFAULT_MAX_POINTS
        );
        assert_eq!(
            history(&c, path, iface, "HistoryMax", Some(u32::MAX)).len(),
            5_000
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn history_is_bounded_on_the_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("history_is_bounded_on_the_bus", INNER, &[]);
}
