//! System appearance from the XDG Desktop Portal (`org.freedesktop.appearance`):
//! colour scheme (light/dark, live) and accent colour. Pure parsing is
//! unit-tested; the D-Bus thread only forwards values.

use std::sync::{Arc, Mutex};

use eframe::egui;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.Settings";
const NS: &str = "org.freedesktop.appearance";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Appearance {
    /// `None`: the portal does not answer, keep the toolkit default.
    pub scheme: Option<Scheme>,
    pub accent: Option<[u8; 3]>,
}

/// `color-scheme`: 0 = no preference (light), 1 = dark, 2 = light.
pub fn parse_scheme(v: u32) -> Scheme {
    if v == 1 {
        Scheme::Dark
    } else {
        Scheme::Light
    }
}

/// `accent-color`: (r, g, b) in 0..=1; anything else is "no accent".
pub fn parse_accent(rgb: (f64, f64, f64)) -> Option<[u8; 3]> {
    let ok = |c: f64| (0.0..=1.0).contains(&c);
    if ok(rgb.0) && ok(rgb.1) && ok(rgb.2) {
        Some([(rgb.0 * 255.0).round() as u8, (rgb.1 * 255.0).round() as u8, (rgb.2 * 255.0).round() as u8])
    } else {
        None
    }
}

fn unwrap_variant(mut v: Value<'_>) -> Value<'_> {
    while let Value::Value(inner) = v {
        v = *inner;
    }
    v
}

fn apply(a: &mut Appearance, key: &str, v: Value<'_>) {
    match (key, unwrap_variant(v)) {
        ("color-scheme", Value::U32(n)) => a.scheme = Some(parse_scheme(n)),
        ("accent-color", v) => {
            a.accent = <(f64, f64, f64)>::try_from(v).ok().and_then(parse_accent);
        }
        _ => {}
    }
}

fn read(conn: &Connection, key: &str) -> Option<Value<'static>> {
    let msg = conn.call_method(Some(DEST), PATH, Some(IFACE), "Read", &(NS, key)).ok()?;
    let v: OwnedValue = msg.body().deserialize().ok()?;
    Some(Value::from(v))
}

pub type Shared = Arc<Mutex<Appearance>>;

/// Follow the portal in the background; repaints the UI on every change.
pub fn spawn(ctx: egui::Context) -> Shared {
    let shared: Shared = Arc::new(Mutex::new(Appearance::default()));
    let s2 = shared.clone();
    let _ = std::thread::Builder::new().name("portal-theme".into()).spawn(move || {
        if let Err(e) = follow(&ctx, &s2) {
            eprintln!("[portal] appearance unavailable ({e}): toolkit default theme");
        }
    });
    shared
}

fn follow(ctx: &egui::Context, shared: &Shared) -> zbus::Result<()> {
    let conn = Connection::session()?;
    let proxy = Proxy::new(&conn, DEST, PATH, IFACE)?;
    let signals = proxy.receive_signal("SettingChanged")?;
    // D-Bus reads on a copy, lock held only to store it: the UI locks the
    // same mutex in every frame and must never wait for the portal (#232).
    let mut a = *shared.lock().unwrap_or_else(|e| e.into_inner());
    for key in ["color-scheme", "accent-color"] {
        if let Some(v) = read(&conn, key) {
            apply(&mut a, key, v);
        }
    }
    *shared.lock().unwrap_or_else(|e| e.into_inner()) = a;
    ctx.request_repaint();
    for msg in signals {
        let Ok((ns, key, value)) = msg.body().deserialize::<(String, String, OwnedValue)>() else {
            continue;
        };
        if ns != NS {
            continue;
        }
        {
            let mut a = shared.lock().unwrap_or_else(|e| e.into_inner());
            apply(&mut a, &key, Value::from(value));
        }
        ctx.request_repaint();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_values() {
        assert_eq!(parse_scheme(0), Scheme::Light);
        assert_eq!(parse_scheme(1), Scheme::Dark);
        assert_eq!(parse_scheme(2), Scheme::Light);
    }

    #[test]
    fn accent_range_checked() {
        assert_eq!(parse_accent((1.0, 0.0, 0.5)), Some([255, 0, 128]));
        assert_eq!(parse_accent((-1.0, 0.0, 0.0)), None);
        assert_eq!(parse_accent((2.0, 0.0, 0.0)), None);
    }

    #[test]
    fn apply_unwraps_nested_variants_and_updates_live() {
        let mut a = Appearance::default();
        apply(&mut a, "color-scheme", Value::Value(Box::new(Value::U32(1))));
        assert_eq!(a.scheme, Some(Scheme::Dark));
        apply(&mut a, "color-scheme", Value::U32(2));
        assert_eq!(a.scheme, Some(Scheme::Light));
        apply(&mut a, "accent-color", Value::from((0.0f64, 1.0f64, 0.0f64)));
        assert_eq!(a.accent, Some([0, 255, 0]));
        apply(&mut a, "accent-color", Value::from((-1.0f64, -1.0f64, -1.0f64)));
        assert_eq!(a.accent, None);
        apply(&mut a, "contrast", Value::U32(1));
    }
}
