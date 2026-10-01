//! #249: the daemon speaks KNotification's dialect to the notification server:
//! hints binding it to `apple-kb-monitor.notifyrc`, buttons, replacement of the
//! previous alert, French texts. Private session bus (`dbus-run-session`) and a
//! fake notification server; no real notification reaches the desktop.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use akm_core::alerts::{Crossing, Urgency};
use akm_core::chemistry::AlertBasis;
use akm_core::link::LinkEvent;
use apple_kb_monitord::notify::{self, Action, Event};
use zbus::zvariant::{OwnedValue, Value};

const INNER: &str = "AKM_NOTIFY_KDE_INNER";
const WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
struct Call {
    id: u32,
    app_name: String,
    replaces_id: u32,
    summary: String,
    actions: Vec<String>,
    hints: HashMap<String, String>,
    timeout: i32,
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
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        _body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let hints = hints
            .into_iter()
            .map(|(k, v)| {
                let s = match &*v {
                    Value::Str(s) => s.to_string(),
                    Value::U8(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    other => format!("{other:?}"),
                };
                (k, s)
            })
            .collect();
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next += 1;
            self.next
        };
        let _ = self.tx.lock().unwrap().send(Call {
            id,
            app_name,
            replaces_id,
            summary,
            actions,
            hints,
            timeout: expire_timeout,
        });
        id
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("fake".into(), "test".into(), "1".into(), "1.2".into())
    }

    fn close_notification(&self, _id: u32) {}
}

fn invoke(from: &zbus::blocking::Connection, id: u32, key: &str) {
    from.emit_signal(
        None::<&str>,
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "ActionInvoked",
        &(id, key),
    )
    .unwrap();
}

fn inner() {
    let (tx, calls): (Sender<Call>, Receiver<Call>) = mpsc::channel();
    let server = zbus::blocking::ConnectionBuilder::session()
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Notifications",
            Fake { tx: Mutex::new(tx), next: 41 },
        )
        .unwrap()
        .build()
        .unwrap();
    let (atx, actions) = mpsc::channel::<(Action, Option<String>)>();
    let atx = Mutex::new(atx);
    notify::set_action_handler(move |a, t| {
        let _ = atx.lock().unwrap().send((a, t));
    });

    // 1. Low-battery threshold: KDE identity, buttons, French (LC_ALL).
    let c = Crossing { threshold: 30, pct: 29.0, urgency: Urgency::Normal };
    notify::battery_crossing(&c, AlertBasis::Estimate);
    let first = calls.recv_timeout(WAIT).expect("first Notify");
    assert_eq!(first.app_name, "Apple Keyboard Monitor");
    assert_eq!(first.replaces_id, 0);
    assert!(first.summary.contains("piles faibles"), "{}", first.summary);
    assert_eq!(first.hints["desktop-entry"], "com.agenceapi.AppleKbMonitor");
    assert_eq!(first.hints["x-kde-appname"], "apple-kb-monitor");
    assert_eq!(first.hints["x-kde-eventId"], "BatteryLow");
    assert_eq!(first.hints["urgency"], "1");
    assert_eq!(first.hints["category"], "device");
    assert_eq!(first.actions, ["default", "Ouvrir", "open", "Ouvrir"]);
    assert_eq!(first.timeout, 12_000);

    // 2. Critical alert replaces it (same id), persistent.
    let c = Crossing { threshold: 5, pct: 4.0, urgency: Urgency::Critical };
    notify::battery_crossing(&c, AlertBasis::Estimate);
    let second = calls.recv_timeout(WAIT).expect("second Notify");
    assert_eq!(second.replaces_id, 42, "a battery alert replaces the previous one");
    assert_eq!(second.hints["x-kde-eventId"], "BatteryCritical");
    assert_eq!(second.hints["urgency"], "2");
    assert_eq!(second.timeout, 0);

    // 3. A rogue client forging ActionInvoked is ignored; the server's counts.
    let rogue = zbus::blocking::Connection::session().unwrap();
    invoke(&rogue, 42, "default");
    assert!(actions.recv_timeout(Duration::from_millis(700)).is_err(), "forged action ran");
    invoke(&server, 42, "default");
    let (a, _) = actions.recv_timeout(WAIT).expect("ActionInvoked default");
    assert_eq!(a, Action::Open);
    // Once is enough: the notification is gone.
    invoke(&server, 42, "default");
    assert!(actions.recv_timeout(Duration::from_millis(700)).is_err());

    // 4. Next battery alert is a new notification again (the old one is gone).
    notify::battery_crossing(&c, AlertBasis::Estimate);
    let third = calls.recv_timeout(WAIT).expect("third Notify");
    assert_eq!(third.replaces_id, 0);

    // 5. Link slot: disconnection then reconnection replace each other;
    //    transient, low urgency.
    let mac = "04:DB:56:CA:42:EE".to_string();
    notify::link(&LinkEvent::Disconnected { mac: mac.clone() });
    let down = calls.recv_timeout(WAIT).unwrap();
    assert_eq!(down.hints["x-kde-eventId"], "KeyboardDisconnected");
    assert_eq!(down.hints["transient"], "true");
    assert_eq!(down.hints["urgency"], "0");
    notify::link(&LinkEvent::Reconnected { mac, pct: Some(80.0) });
    let up = calls.recv_timeout(WAIT).unwrap();
    assert_eq!(up.hints["x-kde-eventId"], "KeyboardReconnected");
    assert_eq!(up.replaces_id, down.id, "reconnection replaces the disconnection");

    // 6. Repair notice: "Repair" button; a button it does not offer is refused.
    notify::notice(Event::RepairNeeded, "Clavier : ré-appairage nécessaire", "b", Urgency::Critical);
    let rep = calls.recv_timeout(WAIT).unwrap();
    assert_eq!(rep.hints["x-kde-eventId"], "RepairNeeded");
    assert_eq!(rep.actions, ["default", "Ouvrir", "repair", "Réparer…", "open", "Ouvrir"]);
    assert_eq!(rep.timeout, 0);
    invoke(&server, rep.id, "ignore");
    assert!(actions.recv_timeout(Duration::from_millis(700)).is_err(), "unoffered button ran");
    invoke(&server, rep.id, "repair");
    assert_eq!(actions.recv_timeout(WAIT).unwrap().0, Action::Repair);

    // 7. Reminder: "Ignore this reminder" is offered and silences the reminder.
    notify::battery_reminder(9.0);
    let rem = calls.recv_timeout(WAIT).unwrap();
    assert_eq!(rem.hints["x-kde-eventId"], "BatteryReminder");
    assert!(rem.actions.contains(&"Ignorer ce rappel".to_string()));
    invoke(&server, rem.id, "ignore");
    assert_eq!(actions.recv_timeout(WAIT).unwrap().0, Action::Ignore);

    // 8. Generic error: no button at all.
    notify::send("Erreur", "x", "dialog-error");
    let err = calls.recv_timeout(WAIT).unwrap();
    assert_eq!(err.hints["x-kde-eventId"], "Error");
    assert!(err.actions.is_empty());
}

#[test]
fn notifications_speak_the_kde_dialect() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    if Command::new("dbus-run-session").arg("--version").output().is_err() {
        eprintln!("SKIP: dbus-run-session not installed");
        return;
    }
    let out = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "notifications_speak_the_kde_dialect", "--nocapture"])
        .env(INNER, "1")
        .env("LC_ALL", "fr_FR.UTF-8")
        .output()
        .expect("run under dbus-run-session");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "inner run failed:\n{text}");
    assert!(text.contains("1 passed"), "inner test did not run:\n{text}");
}
