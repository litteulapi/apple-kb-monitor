//! On a private bus: the process's objects are exported before the name is taken.

use std::sync::Arc;

use akm_core::Watch;
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::service;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::Connection;

const INNER: &str = "AKM_EXPORT_ORDER_INNER";

fn owned(bus: &DBusProxy<'_>) -> bool {
    bus.name_has_owner(service::BUS_NAME.try_into().unwrap())
        .unwrap()
}

fn inner() {
    let probe = Connection::session().unwrap();
    let bus = DBusProxy::new(&probe).unwrap();
    let so = service::ServeOptions::new(Arc::new(Watch::new()), Mailbox::new(), None);
    let mut seen = None;
    let server = service::serve_exporting(&so, |c| {
        seen = Some(owned(&bus));
        c.object_server()
            .at(
                apple_kb_monitord::config_api::PATH,
                apple_kb_monitord::config_api::Settings::default(),
            )
            .unwrap();
    })
    .expect("serve");
    assert_eq!(seen, Some(false), "name taken before the extra objects");
    assert!(owned(&bus));
    let xml: String = probe
        .call_method(
            Some(service::BUS_NAME),
            apple_kb_monitord::config_api::PATH,
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert!(
        xml.contains("com.agenceapi.AppleKbMonitor1.Settings"),
        "{xml}"
    );
    drop(server);
}

#[test]
fn objects_before_the_name() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("objects_before_the_name", INNER, &[("LC_ALL", "C")]);
}
