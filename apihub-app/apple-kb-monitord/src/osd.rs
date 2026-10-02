//! On-screen display of Plasma (`org.kde.osdService`) when the Fn mode
//! changes (#100) and when Caps Lock is pressed (#101).
//!
//! * **Fn mode**: `hid_apple.fnmode` as read in sysfs, looked at when the
//!   daemon changed it (`SetFnMode`), when the published state changes and
//!   when the keyboard is used; a change shows "Fn mode: F1-F12 first".
//! * **Caps Lock**: the state of the keyboard's LED (sysfs, the value the
//!   tray already shows), looked at a moment after the keyboard sent
//!   something; a change shows "Caps Lock on / off". No key is looked at:
//!   the passive listener only says "something arrived"
//!   ([`akm_core::activity`]).
//!
//! The first observation after a start or a reconnection only records the
//! state: an OSD always means "it just changed". Each of the two can be
//! turned off (`[osd] fn_mode`, `[osd] caps_lock`). Nothing is written to
//! the keyboard and nothing is asked from it.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use akm_core::hid_params::Param;
use akm_core::Watch;

use crate::notify::Lang;

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

/// `org.kde.osdService.showText(icon, text)` on the session bus. Without
/// Plasma the call fails and nothing is shown: not an error.
#[derive(Default)]
pub struct PlasmaOsd {
    conn: Mutex<Option<zbus::blocking::Connection>>,
}

impl Sink for PlasmaOsd {
    fn show(&self, icon: &str, text: &str) {
        let mut g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
pub fn fn_mode_text(lang: Lang, mode: i32) -> Option<String> {
    let what = match mode {
        1 => lang.t("media keys first", "touches multim\u{e9}dia d'abord"),
        3 => lang.t(
            "media keys first (auto)",
            "touches multim\u{e9}dia d'abord (auto)",
        ),
        2 => lang.t("F1\u{2013}F12 first", "F1\u{2013}F12 d'abord"),
        0 => lang.t("Fn key has no effect", "touche Fn sans effet"),
        _ => return None,
    };
    Some(match lang {
        Lang::En => format!("Fn mode: {what}"),
        Lang::Fr => format!("Mode Fn\u{a0}: {what}"),
    })
}

/// The `hid_apple.iso_layout` now in effect, in words.
pub fn layout_text(lang: Lang, iso: i32) -> Option<String> {
    let what = match iso {
        1 => "ISO",
        0 => "ANSI",
        -1 => lang.t("automatic", "automatique"),
        _ => return None,
    };
    Some(match lang {
        Lang::En => format!("Keyboard layout: {what}"),
        Lang::Fr => format!("Disposition du clavier\u{a0}: {what}"),
    })
}

pub fn caps_text(lang: Lang, on: bool) -> &'static str {
    match (lang, on) {
        (Lang::En, true) => "Caps Lock on",
        (Lang::En, false) => "Caps Lock off",
        (Lang::Fr, true) => "Verr. Maj activ\u{e9}",
        (Lang::Fr, false) => "Verr. Maj d\u{e9}sactiv\u{e9}",
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
    lang: Lang,
    enabled: Enabled,
    last_fn: Mutex<Option<i32>>,
    last_layout: Mutex<Option<i32>>,
    last_caps: Mutex<Option<bool>>,
}

impl Osd {
    pub fn new(sink: Arc<dyn Sink>, lang: Lang, enabled: Enabled) -> Self {
        Self {
            sink,
            lang,
            enabled,
            last_fn: Mutex::new(None),
            last_layout: Mutex::new(None),
            last_caps: Mutex::new(None),
        }
    }

    /// The Fn mode now read (`None`: `hid_apple` not loaded, nothing known).
    /// Returns whether an OSD was shown.
    pub fn fn_mode(&self, mode: Option<i32>) -> bool {
        let Some(mode) = mode else { return false };
        let mut last = self.last_fn.lock().unwrap_or_else(|e| e.into_inner());
        let changed = last.is_some_and(|l| l != mode);
        *last = Some(mode);
        if !changed || !self.enabled.fn_mode {
            return false;
        }
        match fn_mode_text(self.lang, mode) {
            Some(text) => {
                self.sink.show("input-keyboard", &text);
                true
            }
            None => false,
        }
    }

    /// `hid_apple.iso_layout` now read (`None`: not loaded); shown with the
    /// same switch as the Fn mode. Returns whether an OSD was shown.
    pub fn layout(&self, iso: Option<i32>) -> bool {
        let Some(iso) = iso else { return false };
        let mut last = self.last_layout.lock().unwrap_or_else(|e| e.into_inner());
        let changed = last.is_some_and(|l| l != iso);
        *last = Some(iso);
        if !changed || !self.enabled.fn_mode {
            return false;
        }
        match layout_text(self.lang, iso) {
            Some(text) => {
                self.sink.show("input-keyboard", &text);
                true
            }
            None => false,
        }
    }

    /// The Caps Lock LED now read. Returns whether an OSD was shown.
    pub fn caps(&self, on: bool) -> bool {
        let mut last = self.last_caps.lock().unwrap_or_else(|e| e.into_inner());
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
        self.sink.show(icon, caps_text(self.lang, on));
        true
    }

    /// The keyboard left: the next Caps Lock state seen is a first
    /// observation again (no OSD at the reconnection).
    pub fn disconnected(&self) {
        *self.last_caps.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// What wakes the OSD thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// The keyboard sent something: look at the LED after [`LED_SETTLE`].
    Activity,
    /// The published state changed, or the daemon changed the Fn mode.
    State,
}

static WAKE: OnceLock<SyncSender<Wake>> = OnceLock::new();

/// The daemon just changed the Fn mode: look at it now (no-op until
/// [`spawn`] ran).
pub fn poke() {
    if let Some(tx) = WAKE.get() {
        let _ = tx.try_send(Wake::State);
    }
}

/// One pass: what the thread does when woken. `connected`: a keyboard is
/// connected; `led` reads the Caps Lock LED; `param` reads a `hid_apple`
/// parameter in sysfs.
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

/// Start the OSD thread (`kb-osd`). It sleeps until the keyboard is used or
/// the published state changes; nothing runs while nothing happens.
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
                if let Some(s) = w.wait_newer(seen, Duration::from_secs(3600)) {
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

    fn osd(lang: Lang, fn_mode: bool, caps_lock: bool) -> (Osd, Arc<Rec>) {
        let rec = Arc::new(Rec::default());
        (
            Osd::new(rec.clone(), lang, Enabled { fn_mode, caps_lock }),
            rec,
        )
    }

    fn shown(r: &Rec) -> Vec<(String, String)> {
        r.0.lock().unwrap().clone()
    }

    #[test]
    fn a_change_of_fn_mode_shows_the_new_state_once() {
        let (o, rec) = osd(Lang::Fr, true, true);
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
                    "Mode Fn\u{a0}: F1\u{2013}F12 d'abord".to_string()
                ),
                (
                    "input-keyboard".to_string(),
                    "Mode Fn\u{a0}: touches multim\u{e9}dia d'abord".to_string()
                ),
            ]
        );
        // Unloading hid_apple in between is not a change.
        assert!(!o.fn_mode(None));
        assert!(!o.fn_mode(Some(1)));
    }

    #[test]
    fn caps_lock_shows_on_then_off_and_nothing_at_a_reconnection() {
        let (o, rec) = osd(Lang::En, true, true);
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
        // Keyboard gone with Caps Lock on, back with it off: not a key press.
        o.caps(true);
        o.disconnected();
        assert!(!o.caps(false), "first observation after a reconnection");
        assert_eq!(shown(&rec).len(), 3);
    }

    #[test]
    fn each_osd_can_be_turned_off() {
        let (o, rec) = osd(Lang::En, false, true);
        o.fn_mode(Some(1));
        assert!(!o.fn_mode(Some(2)));
        o.caps(false);
        assert!(o.caps(true));
        assert_eq!(shown(&rec).len(), 1);
        let (o, rec) = osd(Lang::En, true, false);
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
        let (o, rec) = osd(Lang::Fr, true, true);
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
        assert_eq!(layout_text(Lang::En, 0).unwrap(), "Keyboard layout: ANSI");
        assert_eq!(layout_text(Lang::En, 7), None);
        assert_eq!(
            shown(&rec)
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>(),
            [
                "Mode Fn\u{a0}: F1\u{2013}F12 d'abord",
                "Verr. Maj activ\u{e9}",
                "Disposition du clavier\u{a0}: ISO"
            ]
        );
    }

    #[test]
    fn texts_exist_in_both_languages() {
        for m in 0..=3 {
            let (en, fr) = (
                fn_mode_text(Lang::En, m).unwrap(),
                fn_mode_text(Lang::Fr, m).unwrap(),
            );
            assert_ne!(en, fr, "{m}");
        }
        assert_eq!(fn_mode_text(Lang::En, 9), None);
        for on in [true, false] {
            assert_ne!(caps_text(Lang::En, on), caps_text(Lang::Fr, on));
        }
    }

    /// Nothing here can reach the keyboard.
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
