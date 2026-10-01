//! Desktop notifications through `org.freedesktop.Notifications`, in zbus
//! directly (no `notify-rust`: one zbus version in the tree).

use std::collections::HashMap;

use zbus::zvariant::Value;

/// Send a notification on the session bus; errors are logged, never fatal
/// (a headless session has no notification server).
pub fn send(summary: &str, body: &str, icon: &str) {
    let res = (|| -> zbus::Result<u32> {
        let conn = zbus::blocking::Connection::session()?;
        let hints: HashMap<&str, Value<'_>> = HashMap::from([("urgency", Value::U8(2))]);
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

#[cfg(test)]
mod tests {
    #[test]
    fn low_battery_text_rounds() {
        let (s, b) = super::low_battery_text(12.4);
        assert!(s.contains("Low Battery"));
        assert_eq!(b, "Battery at 12% \u{2014} charge soon");
    }
}
