//! On-screen display of Plasma (`org.kde.osdService`) when the Fn mode changes and when Caps
//! Lock is pressed.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use akm_core::hid_params::Param;
use akm_core::Watch;

/// Plasma's OSD service, on the session bus.
pub const OSD_SERVICE: &str = "org.kde.plasmashell";
pub const OSD_PATH: &str = "/org/kde/osdService";
pub const OSD_INTERFACE: &str = "org.kde.osdService";
/// Time left to the host to update the LED after a key press.
pub const LED_SETTLE: Duration = Duration::from_millis(80);

/// Where an OSD is shown (Plasma; a recorder in tests).
pub trait Sink: Send + Sync {
    fn show(&self, icon: &str, text: &str);
}

/// `org.kde.osdService.showText(icon, text)` on the session bus.
#[derive(Default)]
pub struct PlasmaOsd {
    conn: Mutex<Option<zbus::blocking::Connection>>,
}

impl Sink for PlasmaOsd {
    fn show(&self, icon: &str, text: &str) {
        let mut g = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.is_none() {
            *g = zbus::blocking::Connection::session().ok();
        }
        let Some(conn) = g.as_ref() else { return };
        let r = conn.call_method(
            Some(OSD_SERVICE),
            OSD_PATH,
            Some(OSD_INTERFACE),
            "showText",
            &(icon, text),
        );
        if let Err(e) = r {
            tracing::debug!("OSD not shown: {e}");
            if !matches!(e, zbus::Error::MethodError(..)) {
                *g = None; // connection lost: reconnect next time
            }
        }
    }
}

/// What the Fn mode does, in words (`None` = a value that means nothing).
#[must_use]
pub fn fn_mode_text(mode: i32) -> Option<String> {
    let what = akm_core::hid_params::fn_mode_label(mode)?;
    Some(tr!("Fn mode: {what}", what = what))
}

/// The `hid_apple.iso_layout` now in effect, in words.
#[must_use]
pub fn layout_text(iso: i32) -> Option<String> {
    let what = match iso {
        1 => "ISO".to_string(),
        0 => "ANSI".to_string(),
        -1 => tr!("automatic"),
        _ => return None,
    };
    Some(tr!("Keyboard layout: {what}", what = what))
}

#[must_use]
pub fn caps_text(on: bool) -> String {
    if on {
        tr!("Caps Lock on")
    } else {
        tr!("Caps Lock off")
    }
}

/// Which OSDs are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enabled {
    pub fn_mode: bool,
    pub caps_lock: bool,
}

/// Remembers the last state seen and shows an OSD on a change.
pub struct Osd {
    sink: Arc<dyn Sink>,
    enabled: Enabled,
    last_fn: Mutex<Option<i32>>,
    last_layout: Mutex<Option<i32>>,
    last_caps: Mutex<Option<bool>>,
}

impl Osd {
    pub fn new(sink: Arc<dyn Sink>, enabled: Enabled) -> Self {
        Self {
            sink,
            enabled,
            last_fn: Mutex::new(None),
            last_layout: Mutex::new(None),
            last_caps: Mutex::new(None),
        }
    }

    /// The Fn mode now read (`None`: `hid_apple` not loaded, nothing known).
    pub fn fn_mode(&self, mode: Option<i32>) -> bool {
        let Some(mode) = mode else { return false };
        let mut last = self
            .last_fn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = last.is_some_and(|l| l != mode);
        *last = Some(mode);
        if !changed || !self.enabled.fn_mode {
            return false;
        }
        match fn_mode_text(mode) {
            Some(text) => {
                self.sink.show("input-keyboard", &text);
                true
            }
            None => false,
        }
    }

    /// `hid_apple.iso_layout` now read; shown with the same switch as the Fn mode.
    pub fn layout(&self, iso: Option<i32>) -> bool {
        let Some(iso) = iso else { return false };
        let mut last = self
            .last_layout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = last.is_some_and(|l| l != iso);
        *last = Some(iso);
        if !changed || !self.enabled.fn_mode {
            return false;
        }
        match layout_text(iso) {
            Some(text) => {
                self.sink.show("input-keyboard", &text);
                true
            }
            None => false,
        }
    }

    /// The Caps Lock LED now read.
    pub fn caps(&self, on: bool) -> bool {
        let mut last = self
            .last_caps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = last.is_some_and(|l| l != on);
        *last = Some(on);
        if !changed || !self.enabled.caps_lock {
            return false;
        }
        let icon = if on {
            "input-caps-on"
        } else {
            "input-keyboard"
        };
        self.sink.show(icon, &caps_text(on));
        true
    }

    /// The keyboard left: the next Caps Lock state seen is a first observation again.
    pub fn disconnected(&self) {
        *self
            .last_caps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// The keyboard sent something: look at the LED after [`LED_SETTLE`].
    Activity,
    /// The published state changed, or the daemon changed the Fn mode.
    State,
}

static WAKE: OnceLock<SyncSender<Wake>> = OnceLock::new();

/// The daemon just changed the Fn mode: look at it now (no-op until [`spawn`] ran).
pub fn poke() {
    if let Some(tx) = WAKE.get() {
        let _ = tx.try_send(Wake::State);
    }
}

/// One pass: what the thread does when woken.
pub fn pass(
    osd: &Osd,
    connected: bool,
    led: &dyn Fn() -> bool,
    param: &dyn Fn(Param) -> Option<i32>,
) {
    osd.fn_mode(param(Param::FnMode));
    osd.layout(param(Param::IsoLayout));
    if connected {
        osd.caps(led());
    } else {
        osd.disconnected();
    }
}

/// Start the OSD thread (`kb-osd`).
pub fn spawn(osd: Osd, watch: Arc<Watch>) {
    if !osd.enabled.fn_mode && !osd.enabled.caps_lock {
        tracing::info!("OSD off ([osd] fn_mode and caps_lock are false)");
        return;
    }
    let (tx, rx): (SyncSender<Wake>, Receiver<Wake>) = mpsc::sync_channel(4);
    let _ = WAKE.set(tx.clone());
    let atx = tx.clone();
    akm_core::activity::subscribe(move || {
        let _ = atx.try_send(Wake::Activity);
    });
    let w = watch.clone();
    let _ = std::thread::Builder::new()
        .name("kb-osd-watch".into())
        .spawn(move || {
            let mut seen = w.version();
            loop {
                if let Some(s) = w.wait_newer(seen, Duration::from_hours(1)) {
                    seen = s.version;
                    if tx.send(Wake::State).is_err() {
                        return;
                    }
                }
            }
        });
    let _ = std::thread::Builder::new()
        .name("kb-osd".into())
        .spawn(move || {
            let read_fn =
                |p: Param| p.read_in(std::path::Path::new(akm_core::hid_params::SYSFS_DIR));
            // First observation: the state at start, without an OSD.
            pass(
                &osd,
                watch.get().connected,
                &|| akm_core::led::read_led_state().0,
                &read_fn,
            );
            for wake in rx {
                if wake == Wake::Activity {
                    std::thread::sleep(LED_SETTLE);
                }
                pass(
                    &osd,
                    watch.get().connected,
                    &|| akm_core::led::read_led_state().0,
                    &read_fn,
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Rec(Mutex<Vec<(String, String)>>);
    impl Sink for Rec {
        fn show(&self, icon: &str, text: &str) {
            self.0.lock().unwrap().push((icon.into(), text.into()));
        }
    }

    fn osd(fn_mode: bool, caps_lock: bool) -> (Osd, Arc<Rec>) {
        let rec = Arc::new(Rec::default());
        (Osd::new(rec.clone(), Enabled { fn_mode, caps_lock }), rec)
    }

    fn shown(r: &Rec) -> Vec<(String, String)> {
        r.0.lock().unwrap().clone()
    }

    #[test]
    fn a_change_of_fn_mode_shows_the_new_state_once() {
        let (o, rec) = osd(true, true);
        assert!(!o.fn_mode(None), "hid_apple not loaded");
        assert!(!o.fn_mode(Some(1)), "first observation: no OSD");
        assert!(!o.fn_mode(Some(1)), "unchanged");
        assert!(o.fn_mode(Some(2)));
        assert!(!o.fn_mode(Some(2)), "once per change");
        assert!(o.fn_mode(Some(1)));
        assert_eq!(
            shown(&rec),
            [
                (
                    "input-keyboard".to_string(),
                    "Fn mode: F1\u{2013}F12 first".to_string()
                ),
                (
                    "input-keyboard".to_string(),
                    "Fn mode: media keys first".to_string()
                ),
            ]
        );
        assert!(!o.fn_mode(None));
        assert!(!o.fn_mode(Some(1)));
    }

    #[test]
    fn caps_lock_shows_on_then_off_and_nothing_at_a_reconnection() {
        let (o, rec) = osd(true, true);
        assert!(!o.caps(false), "state at start");
        assert!(o.caps(true));
        assert!(!o.caps(true));
        assert!(o.caps(false));
        assert_eq!(
            shown(&rec),
            [
                ("input-caps-on".to_string(), "Caps Lock on".to_string()),
                ("input-keyboard".to_string(), "Caps Lock off".to_string()),
            ]
        );
        o.caps(true);
        o.disconnected();
        assert!(!o.caps(false), "first observation after a reconnection");
        assert_eq!(shown(&rec).len(), 3);
    }

    #[test]
    fn each_osd_can_be_turned_off() {
        let (o, rec) = osd(false, true);
        o.fn_mode(Some(1));
        assert!(!o.fn_mode(Some(2)));
        o.caps(false);
        assert!(o.caps(true));
        assert_eq!(shown(&rec).len(), 1);
        let (o, rec) = osd(true, false);
        o.caps(false);
        assert!(!o.caps(true));
        o.fn_mode(Some(1));
        assert!(o.fn_mode(Some(2)));
        assert_eq!(
            shown(&rec),
            [(
                "input-keyboard".to_string(),
                "Fn mode: F1\u{2013}F12 first".to_string()
            )]
        );
    }

    #[test]
    fn a_pass_reads_the_led_only_while_a_keyboard_is_connected() {
        let (o, rec) = osd(true, true);
        let led_reads = std::cell::Cell::new(0);
        let led = |v: bool| {
            led_reads.set(led_reads.get() + 1);
            v
        };
        let param = |fnmode: i32, iso: i32| {
            move |p: Param| match p {
                Param::FnMode => Some(fnmode),
                Param::IsoLayout => Some(iso),
                Param::SwapOptCmd => None,
            }
        };
        pass(&o, false, &|| led(true), &param(1, -1));
        assert_eq!(led_reads.get(), 0, "disconnected: the LED is not read");
        pass(&o, true, &|| led(false), &param(1, -1));
        pass(&o, true, &|| led(true), &param(2, -1));
        pass(&o, true, &|| led(true), &param(2, 1));
        assert_eq!(led_reads.get(), 3);
        assert_eq!(layout_text(0).unwrap(), "Keyboard layout: ANSI");
        assert_eq!(layout_text(7), None);
        assert_eq!(
            shown(&rec)
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>(),
            [
                "Fn mode: F1\u{2013}F12 first",
                "Caps Lock on",
                "Keyboard layout: ISO"
            ]
        );
    }

    #[test]
    fn the_osd_never_asks_the_keyboard_anything() {
        let src = include_str!("osd.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "hidraw",
            "set_led",
            "read_keyboard",
            "flash_capslock",
            "write(",
        ] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
    }
}
