//! D-Bus API v2 on a PRIVATE session bus.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akm_core::alerts::{Crossing, Urgency};
use akm_core::batteries::Replacement;
use akm_core::forecast::Forecast;
use akm_core::hid_params::Param;
use akm_core::history::{History, SystemClock};
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::alias::AliasBackend;
use apple_kb_monitord::devices::{self, DEVICE_INTERFACE};
use apple_kb_monitord::events::{DeviceEvent, EventHub};
use apple_kb_monitord::service::{self, ServeOptions};
use apple_kb_monitord::settings::{SetError, SettingsBackend};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::MatchRule;

const INNER: &str = "AKM_DBUS_V2_INNER";
const MAC: &str = "AA:BB:CC:DD:EE:F1";
const DEV: &str = "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1";

#[derive(Default)]
struct FakeSettings(Mutex<HashMap<&'static str, i32>>);
impl SettingsBackend for FakeSettings {
    fn get(&self, p: Param) -> i32 {
        *self.0.lock().unwrap().get(p.name()).unwrap_or(&1)
    }
    fn apply(&self, p: Param, v: i32) -> Result<(), SetError> {
        self.0.lock().unwrap().insert(p.name(), v);
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FakeAlias(Mutex<Vec<(String, String)>>);
impl AliasBackend for FakeAlias {
    fn get(&self, _: &str) -> Option<String> {
        self.0.lock().unwrap().last().map(|(_, a)| a.clone())
    }
    fn set(&self, mac: &str, alias: &str) -> Result<(), SetError> {
        self.0.lock().unwrap().push((mac.into(), alias.into()));
        Ok(())
    }
}

fn keyboard(pct: f64, now: u64) -> Snapshot {
    let mut k = KbReport::default();
    k.battery.percentage_fine = Some(pct);
    k.battery.voltage = Some(2.81);
    k.device.mac = Some(MAC.into());
    k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
    k.firmware.version = Some("0x0050".into());
    akm_core::firmware::assess_report(Some(0x0239), &mut k.firmware);
    Snapshot {
        connected: true,
        keyboard: Some(k),
        last_update: now,
        forecast: Some(Forecast {
            rate_pct_per_day: 1.0,
            empty_at: now + 90 * 86_400,
            fitted_pct: pct,
            span_s: 5 * 86_400,
            buckets: 60,
        }),
        batteries_installed_at: Some(now - 10 * 86_400),
        ..Default::default()
    }
}

fn get(c: &Connection, path: &str, iface: &str, prop: &str) -> OwnedValue {
    c.call_method(
        Some(service::BUS_NAME),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(iface, prop),
    )
    .unwrap_or_else(|e| panic!("Get {prop}: {e}"))
    .body()
    .deserialize()
    .unwrap()
}

fn call0(c: &Connection, path: &str, method: &str) -> zbus::Result<zbus::Message> {
    c.call_method(
        Some(service::BUS_NAME),
        path,
        Some(DEVICE_INTERFACE),
        method,
        &(),
    )
}

fn set_alias_dev(c: &Connection, name: &str) -> zbus::Result<zbus::Message> {
    c.call_method(
        Some(service::BUS_NAME),
        DEV,
        Some(DEVICE_INTERFACE),
        "SetAlias",
        &(name,),
    )
}

fn set_alias_root(c: &Connection, mac: &str, name: &str) -> zbus::Result<zbus::Message> {
    c.call_method(
        Some(service::BUS_NAME),
        service::OBJECT_PATH,
        Some(service::INTERFACE),
        "SetAlias",
        &(mac, name),
    )
}

fn call_i(c: &Connection, path: &str, method: &str, v: i32) -> zbus::Result<zbus::Message> {
    c.call_method(
        Some(service::BUS_NAME),
        path,
        Some(DEVICE_INTERFACE),
        method,
        &(v,),
    )
}

fn collect(
    it: MessageIterator,
    mut pred: impl FnMut(&str, &zbus::Message) -> bool + Send + 'static,
) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for m in it {
            let Ok(m) = m else { continue };
            let member = m
                .header()
                .member()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            if pred(&member, &m) {
                let _ = tx.send(());
                return;
            }
        }
    });
    rx
}

fn subscribe(path: &str) -> MessageIterator {
    let sub = Connection::session().unwrap();
    // Listening before the rule is added: once AddMatch has answered, no signal is missed.
    let it = MessageIterator::from(sub.clone());
    let proxy = zbus::blocking::fdo::DBusProxy::new(&sub).unwrap();
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .path(path.to_string())
        .unwrap()
        .build();
    proxy.add_match_rule(rule).unwrap();
    it
}

#[allow(clippy::too_many_lines)] // one end-to-end scenario, read top to bottom
#[allow(clippy::many_single_char_names)] // test fixtures with short local names
fn inner() {
    let now = 1_790_000_000 + 10 * 86_400;
    let dir = std::env::temp_dir().join(format!("akm-dbus-v2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A new set in ANOTHER keyboard: never one of this device's sets.
    let other =
        r#"{"ts":1790500000,"pct":100.0,"event":"battery_replaced","mac":"11:22:33:44:55:66"}"#;
    std::fs::write(
        dir.join("h.jsonl"),
        format!(
            "{}{other}\n",
            include_str!("../../akm-core/tests/fixtures/history_replacements.jsonl")
        ),
    )
    .unwrap();

    let watch = Arc::new(Watch::new());
    watch.publish(keyboard(90.0, now));
    let events = EventHub::new();
    let fake = Arc::new(FakeSettings::default());
    let mut so = ServeOptions::new(
        watch.clone(),
        Mailbox::new(),
        Some(Arc::new(History::new(dir.join("h.jsonl"), SystemClock))),
    );
    so.events = events.clone();
    so.settings = fake.clone();
    let fake_alias = Arc::new(FakeAlias::default());
    so.alias = fake_alias.clone();
    let _server = service::serve_with(&so).expect("serve");

    let c = Connection::session().unwrap();

    for path in [service::OBJECT_PATH, DEV] {
        let xml: String = c
            .call_method(
                Some(service::BUS_NAME),
                path,
                Some("org.freedesktop.DBus.Introspectable"),
                "Introspect",
                &(),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert_xml_comments_well_formed(path, &xml);
        // A countdown changes every second without a signal: introspection must say so.
        let rem = &xml[xml.find("<property name=\"RemainingSeconds\"").expect(path)..];
        let rem = &rem[..rem.find("</property>").unwrap()];
        assert!(
            rem.contains(
                r#"name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false""#
            ),
            "{path}: {rem}"
        );
        if path == service::OBJECT_PATH {
            for m in [
                "RereadName",
                "Diagnose",
                "History",
                "GetDevices",
                "BatterySets",
            ] {
                assert!(
                    xml.contains(&format!("<method name=\"{m}\"")),
                    "{m} missing"
                );
            }
        }
    }

    assert_eq!(
        i32::try_from(get(&c, service::OBJECT_PATH, service::INTERFACE, "Battery")).unwrap(),
        90
    );
    assert_eq!(
        u32::try_from(get(
            &c,
            service::OBJECT_PATH,
            service::INTERFACE,
            "InterfaceVersion"
        ))
        .unwrap(),
        2
    );
    let rem = i64::try_from(get(
        &c,
        service::OBJECT_PATH,
        service::INTERFACE,
        "RemainingSeconds",
    ))
    .unwrap();
    assert!(rem > 0, "RemainingSeconds {rem}");

    for (path, iface) in [
        (service::OBJECT_PATH, service::INTERFACE),
        (DEV, DEVICE_INTERFACE),
    ] {
        let s = |p: &str| String::try_from(get(&c, path, iface, p)).unwrap();
        assert_eq!(s("FirmwareVersion"), "0x0050", "{path}");
        assert_eq!(s("FirmwareLatestKnown"), "0x0050", "{path}");
        assert_eq!(s("FirmwareStatus"), "up_to_date", "{path}");
    }

    let paths: Vec<OwnedObjectPath> = c
        .call_method(
            Some(service::BUS_NAME),
            service::OBJECT_PATH,
            Some(service::INTERFACE),
            "GetDevices",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert_eq!(
        paths.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        vec![DEV]
    );
    assert_eq!(devices::device_path(MAC).unwrap().as_str(), DEV);
    let managed: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> = c
        .call_method(
            Some(service::BUS_NAME),
            service::OBJECT_PATH,
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let dev = managed
        .iter()
        .find(|(p, _)| p.as_str() == DEV)
        .map(|(_, v)| v)
        .expect("device in GetManagedObjects");
    let props = dev.get(DEVICE_INTERFACE).expect("Device interface");
    assert_eq!(
        String::try_from(props.get("Mac").unwrap().try_clone().unwrap()).unwrap(),
        MAC
    );
    assert_eq!(
        i32::try_from(get(&c, DEV, DEVICE_INTERFACE, "Battery")).unwrap(),
        90
    );
    assert!(bool::try_from(get(&c, DEV, DEVICE_INTERFACE, "Connected")).unwrap());
    let r = i64::try_from(get(&c, DEV, DEVICE_INTERFACE, "RemainingSeconds")).unwrap();
    assert!(r > 0);
    let rate = f64::try_from(get(&c, DEV, DEVICE_INTERFACE, "DischargeRate")).unwrap();
    assert!((rate - 1.0).abs() < 1e-9, "{rate}");
    assert_eq!(
        u64::try_from(get(&c, DEV, DEVICE_INTERFACE, "BatteriesInstalledAt")).unwrap(),
        now - 10 * 86_400
    );

    let sets: String = call0(&c, DEV, "BatterySets")
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&sets).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3, "{sets}");

    assert_eq!(
        i32::try_from(get(&c, DEV, DEVICE_INTERFACE, "FnMode")).unwrap(),
        1
    );
    let it = subscribe(DEV);
    // A daemon change re-announces the three keyboard parameters.
    let mut missing = vec!["FnMode", "SwapOptCmd", "IsoLayout"];
    let fn_changed = collect(it, move |member, m| {
        if member != "PropertiesChanged" {
            return false;
        }
        let (_, changed, invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) =
            m.body().deserialize().unwrap();
        missing.retain(|p| !changed.contains_key(*p) && !invalidated.iter().any(|s| s == p));
        missing.is_empty()
    });
    call_i(&c, DEV, "SetFnMode", 2).unwrap();
    assert_eq!(
        i32::try_from(get(&c, DEV, DEVICE_INTERFACE, "FnMode")).unwrap(),
        2
    );
    fn_changed
        .recv_timeout(Duration::from_secs(5))
        .expect("FnMode PropertiesChanged");
    for m in ["SetIsoLayout", "SetSwapOptCmd"] {
        let e = call_i(&c, DEV, m, 1).unwrap_err();
        assert!(e.to_string().contains("UnknownMethod"), "{m}: {e}");
    }
    let e = call_i(&c, DEV, "SetFnMode", 9).unwrap_err();
    assert!(e.to_string().contains("InvalidArgs"), "{e}");
    assert_eq!(*fake.0.lock().unwrap(), HashMap::from([("fnmode", 2)]));

    // Changed outside the daemon (akmctl, sysfs): announced once the daemon reads it again.
    let iso_changed = collect(subscribe(DEV), |member, m| {
        member == "PropertiesChanged"
            && m.body()
                .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                .is_ok_and(|(_, c, _)| {
                    c.get("IsoLayout")
                        .is_some_and(|v| i32::try_from(v).ok() == Some(0))
                })
    });
    fake.0.lock().unwrap().insert(Param::IsoLayout.name(), 0);
    watch.publish(keyboard(89.0, now));
    iso_changed
        .recv_timeout(Duration::from_secs(5))
        .expect("IsoLayout PropertiesChanged after an outside change");
    fake.0.lock().unwrap().remove(Param::IsoLayout.name());

    let r: String = set_alias_dev(&c, "  Desk  ")
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert_eq!(r, "Desk");
    let r: String = set_alias_root(&c, MAC, "")
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert_eq!(r, "", "reset reaches the backend as an empty alias");
    for bad in ["a\nb", "x\u{202E}y", &"z".repeat(65)] {
        let e = set_alias_dev(&c, bad).unwrap_err();
        assert!(e.to_string().contains("InvalidArgs"), "{bad:?}: {e}");
    }
    let e = set_alias_root(&c, "zz", "x").unwrap_err();
    assert!(e.to_string().contains("InvalidArgs"), "{e}");
    assert_eq!(
        *fake_alias.0.lock().unwrap(),
        vec![
            (MAC.to_string(), "Desk".to_string()),
            (MAC.to_string(), String::new())
        ]
    );
    assert_eq!(
        String::try_from(get(&c, DEV, DEVICE_INTERFACE, "Name")).unwrap(),
        ""
    );
    let mut named = keyboard(90.0, now);
    named.keyboard.as_mut().unwrap().device.alias = Some("Desk".into());
    watch.publish(named);
    let deadline = Instant::now() + Duration::from_secs(2);
    while String::try_from(get(&c, service::OBJECT_PATH, service::INTERFACE, "Name")).unwrap()
        != "Desk"
    {
        assert!(Instant::now() < deadline, "Name not published");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        String::try_from(get(&c, DEV, DEVICE_INTERFACE, "Name")).unwrap(),
        "Desk"
    );
    watch.publish(keyboard(90.0, now));

    let it = subscribe(DEV);
    let got = Arc::new(Mutex::new(Vec::<String>::new()));
    let g2 = got.clone();
    let done = collect(it, move |member, m| {
        let mut g = g2.lock().unwrap();
        match member {
            "BatteryLevelCrossed" => {
                let (t, b, u): (u32, i32, String) = m.body().deserialize().unwrap();
                g.push(format!("crossed {t} {b} {u}"));
            }
            "BatteryReplaced" => {
                let (ts, b0, b1, _v0, v1): (u64, i32, i32, f64, f64) =
                    m.body().deserialize().unwrap();
                g.push(format!("replaced {ts} {b0} {b1} {v1}"));
            }
            "ConnectionChanged" => {
                let (c, b): (bool, i32) = m.body().deserialize().unwrap();
                g.push(format!("connection {c} {b}"));
            }
            _ => {}
        }
        g.len() == 3
    });
    events.publish(&DeviceEvent::BatteryLevelCrossed {
        mac: MAC.into(),
        crossing: Crossing {
            threshold: 30,
            pct: 29.4,
            urgency: Urgency::Normal,
        },
    });
    events.publish(&DeviceEvent::BatteryReplaced {
        mac: MAC.into(),
        replacement: Replacement {
            ts: now,
            pct_before: Some(4.0),
            pct_after: 100.0,
            voltage_before: Some(2.1),
            voltage_after: Some(3.05),
        },
    });
    watch.publish(Snapshot::default()); // keyboard gone
    done.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| panic!("signals: {:?}", got.lock().unwrap()));
    let g = got.lock().unwrap().clone();
    assert!(g.contains(&"crossed 30 29 normal".to_string()), "{g:?}");
    assert!(g.contains(&format!("replaced {now} 4 100 3.05")), "{g:?}");
    assert!(g.contains(&"connection false -1".to_string()), "{g:?}");

    let deadline = Instant::now() + Duration::from_secs(2);
    while bool::try_from(get(&c, DEV, DEVICE_INTERFACE, "Connected")).unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        i32::try_from(get(&c, DEV, DEVICE_INTERFACE, "Battery")).unwrap(),
        -1
    );
    assert_eq!(
        i64::try_from(get(&c, DEV, DEVICE_INTERFACE, "RemainingSeconds")).unwrap(),
        -1
    );
    assert!(String::try_from(get(&c, DEV, DEVICE_INTERFACE, "Model"))
        .unwrap()
        .contains("A1314"));

    let other = Connection::session().unwrap();
    let w2 = Arc::new(Watch::new());
    w2.publish(keyboard(50.0, now));
    let silent = Mailbox::new();
    let (tx, _never_answered) = mpsc::channel();
    silent.install(tx);
    let mut so = ServeOptions::new(w2, silent, None);
    so.settings = Arc::new(apple_kb_monitord::settings::HelperBackend::with_paths(
        "/usr/bin/pkexec",
        "/nonexistent/akm-helper",
    ));
    so.bus_name = "com.agenceapi.AppleKbMonitor1.Test".into();
    let shared = service::export_on(&other, &so).unwrap();
    devices::ensure_device(&other, &shared, MAC, "x", "").unwrap();
    let e = c
        .call_method(
            Some("com.agenceapi.AppleKbMonitor1.Test"),
            DEV,
            Some(DEVICE_INTERFACE),
            "SetFnMode",
            &(2i32,),
        )
        .unwrap_err();
    assert!(e.to_string().contains("NotSupported"), "{e}");

    // Device.Refresh waits up to 2 s for the actor: the bus executor must keep answering.
    let slow = std::thread::spawn(|| {
        let c = Connection::session().unwrap();
        let _ = c.call_method(
            Some("com.agenceapi.AppleKbMonitor1.Test"),
            DEV,
            Some(DEVICE_INTERFACE),
            "Refresh",
            &(),
        );
    });
    std::thread::sleep(Duration::from_millis(200));
    let (done, answered) = mpsc::channel();
    let c2 = c.clone();
    std::thread::spawn(move || {
        let r = c2.call_method(
            Some("com.agenceapi.AppleKbMonitor1.Test"),
            DEV,
            Some(DEVICE_INTERFACE),
            "History",
            &(0u64,),
        );
        let _ = done.send(r.is_ok());
    });
    let ok = answered
        .recv_timeout(Duration::from_secs(1))
        .expect("History waited behind Device.Refresh");
    assert!(ok);
    slow.join().unwrap();

    events.publish(&DeviceEvent::Forgotten { mac: MAC.into() });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let paths: Vec<OwnedObjectPath> = c
            .call_method(
                Some(service::BUS_NAME),
                service::OBJECT_PATH,
                Some(service::INTERFACE),
                "GetDevices",
                &(),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        if paths.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "still exported: {paths:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(c
        .call_method(
            Some(service::BUS_NAME),
            DEV,
            Some(DEVICE_INTERFACE),
            "Refresh",
            &()
        )
        .is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

fn assert_xml_comments_well_formed(path: &str, xml: &str) {
    let mut rest = xml;
    while let Some(i) = rest.find("<!--") {
        let body = &rest[i + 4..];
        let end = body.find("-->").expect("unterminated comment");
        let c = &body[..end];
        assert!(
            !c.contains("--") && !c.ends_with('-'),
            "{path}: comment not well formed: {c:?}"
        );
        rest = &body[end + 3..];
    }
}

#[test]
fn api_v2_on_private_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("api_v2_on_private_bus", INNER, &[]);
}
