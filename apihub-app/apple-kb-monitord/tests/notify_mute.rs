//! #162: a notification server that owns the name but never answers must not
//! block the caller. Private session bus (`dbus-daemon` without service directory, see `testbus`).

use std::time::{Duration, Instant};

use akm_core::link::LinkEvent;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct T(std::thread::Thread);
    impl Wake for T {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(T(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

const INNER: &str = "AKM_NOTIFY_MUTE_INNER";

fn inner() {
    // Mute server: owns the name, its executor is never ticked.
    let _server = block_on(
        zbus::ConnectionBuilder::session()
            .unwrap()
            .internal_executor(false)
            .name("org.freedesktop.Notifications")
            .unwrap()
            .build(),
    )
    .unwrap();
    // Control: the server really is mute (the blocking call does not return).
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        apple_kb_monitord::notify::send_blocking(
            "s",
            "b",
            "i",
            akm_core::alerts::Urgency::Low,
            true,
        );
        let _ = tx.send(());
    });
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_err(), "server answered");
    let t = Instant::now();
    for _ in 0..3 {
        apple_kb_monitord::notify::link(&LinkEvent::Disconnected {
            mac: "AA:BB:CC:DD:EE:F1".into(),
        });
        apple_kb_monitord::notify::send("s", "b", "battery-caution");
    }
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "notify blocked the caller for {:?}",
        t.elapsed()
    );
}

#[test]
fn mute_notification_server_does_not_block() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    let Some((_bus, mut cmd)) = apple_kb_monitord::testbus::rerun() else {
        eprintln!("SKIP: dbus-daemon not installed");
        return;
    };
    let out = cmd
        .args(["--exact", "mute_notification_server_does_not_block", "--nocapture"])
        .env(INNER, "1")
        .output()
        .expect("run on the private bus");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "inner run failed:\n{text}");
    assert!(text.contains("1 passed"), "inner test did not run:\n{text}");
}
