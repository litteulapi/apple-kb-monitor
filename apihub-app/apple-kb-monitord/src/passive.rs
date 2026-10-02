//! Passive input-report listening exposed on D-Bus (#130, #187, #129).
//!
//! A dedicated thread (`kb-passive`, see [`akm_core::passive`]) reads the
//! interrupt reports the keyboard sends by itself on its hidraw node; nothing
//! is ever requested from the keyboard (no GET_REPORT / SET_REPORT / LED write)
//! and the acquisition thread is never involved. The result is published on
//! the keyboard's v2 object
//! (`/com/agenceapi/AppleKbMonitor1/devices/<MAC>`) as a second interface,
//! `com.agenceapi.AppleKbMonitor1.Input`:
//!
//! Properties (`PropertiesChanged` emitted):
//! * `Listening` b: a hidraw node is being listened to
//! * `FnLock` i: last `0x05` state byte, -1 unknown
//! * `LastSleepEvent` t: unix time of the last `0x04` report, 0 never
//! * `WakeCount` t: `0x13` events (ready / connection request) since start
//! * `EjectPressed` b, `FnPressed` b: from `0x11`
//! * `MediaKeys` u: `0x12` bit field held
//! * `BatteryStatus` i: last `0x30` byte, -1 unknown
//! * `PoweredOff` b: the keyboard announced its switch-off (`0x13` bit 1 = 0)
//!   and has not been seen powered on since (#190)
//! * `State` s: the whole state as JSON (read by `akmctl status --json`)
//!
//! Signals: `SleepEvent(t ts, y code)`, `Wake(b ready, b conn_request, t count)`,
//! `EjectChanged(b pressed)`, `FnLockUpdated(i value)`,
//! `KeyboardOff(t ts)` (the keyboard says it switches off, distinct from a lost
//! link, #190), `BatteryAlert(s state)` (`low` / `critical`, announced by the
//! keyboard itself, #189).
//!
//! Only state and timestamps leave the thread, never key codes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use akm_core::alerts;
use akm_core::passive::{self, Config, Msg, PassiveEvent, PassiveState};
use akm_core::Watch;
use zbus::blocking::Connection;
use zbus::interface;
use zbus::object_server::SignalContext;

use crate::devices::device_path;

pub const INPUT_INTERFACE: &str = "com.agenceapi.AppleKbMonitor1.Input";

/// Raise a desktop notification when the keyboard announces a low / critical
/// battery (`0x30`, #189). Off until `main` enables it (`notify` and
/// `alerts_enabled` of the configuration), so tests never notify.
static KEYBOARD_ALERTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_keyboard_alerts(on: bool) {
    KEYBOARD_ALERTS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Second source of input events: the battery state the daemon READS (GET
/// Input `0x30` after `0x47`, Apple's R2, #251) enters the same publisher as
/// the pushed `A1 30 xx`, so the state, the D-Bus properties and the alerts
/// of #189 are deduplicated in one place ([`passive::battery_state_alert`]
/// only fires on a rise).
static INJECT: std::sync::OnceLock<Mutex<mpsc::Sender<Msg>>> = std::sync::OnceLock::new();

/// Hand a decoded input report to the publisher; false when no listener runs.
pub fn inject(ev: PassiveEvent) -> bool {
    INJECT.get().is_some_and(|tx| {
        tx.lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Msg::Event(ev))
            .is_ok()
    })
}

/// The listener thread was started: key presses reach
/// [`akm_core::read_policy::note_input`], so "no key since the wake" means
/// something. False (no bus, start failed) = nobody tells about key presses.
static LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn listener_started() -> bool {
    LISTENING.load(std::sync::atomic::Ordering::Relaxed)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

type Shared = Arc<Mutex<PassiveState>>;

fn lock(s: &Shared) -> PassiveState {
    s.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The `Input` interface of one keyboard object.
pub struct Input {
    state: Shared,
}

#[interface(name = "com.agenceapi.AppleKbMonitor1.Input")]
impl Input {
    #[zbus(property)]
    fn listening(&self) -> bool {
        lock(&self.state).listening
    }
    /// Last Fn-lock byte (`0x05`), -1 unknown.
    #[zbus(property)]
    fn fn_lock(&self) -> i32 {
        lock(&self.state).fn_lock.map_or(-1, i32::from)
    }
    /// Unix time of the last sleep notification (`0x04`), 0 never.
    #[zbus(property)]
    fn last_sleep_event(&self) -> u64 {
        lock(&self.state).last_sleep_ts
    }
    #[zbus(property)]
    fn wake_count(&self) -> u64 {
        lock(&self.state).wake_count
    }
    #[zbus(property)]
    fn eject_pressed(&self) -> bool {
        lock(&self.state).eject_pressed
    }
    #[zbus(property)]
    fn fn_pressed(&self) -> bool {
        lock(&self.state).fn_pressed
    }
    #[zbus(property)]
    fn media_keys(&self) -> u32 {
        u32::from(lock(&self.state).media_bits)
    }
    /// Last battery status byte (`0x30`), -1 unknown.
    #[zbus(property)]
    fn battery_status(&self) -> i32 {
        lock(&self.state).batt_stat.map_or(-1, i32::from)
    }
    /// The keyboard announced its switch-off and has not come back (#190).
    #[zbus(property)]
    fn powered_off(&self) -> bool {
        lock(&self.state).powered_off
    }
    #[zbus(property)]
    fn state(&self) -> String {
        lock(&self.state).to_json().to_string()
    }

    #[zbus(signal)]
    async fn sleep_event(ctxt: &SignalContext<'_>, ts: u64, code: u8) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn wake(
        ctxt: &SignalContext<'_>,
        ready: bool,
        conn_request: bool,
        count: u64,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn eject_changed(ctxt: &SignalContext<'_>, pressed: bool) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn fn_lock_updated(ctxt: &SignalContext<'_>, value: i32) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn keyboard_off(ctxt: &SignalContext<'_>, ts: u64) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn battery_alert(ctxt: &SignalContext<'_>, state: &str) -> zbus::Result<()>;
}

/// Names of the properties that differ between two states.
pub fn changed_props(o: &PassiveState, n: &PassiveState) -> Vec<&'static str> {
    let mut v = vec![];
    let mut ch = |c: bool, name: &'static str| {
        if c {
            v.push(name)
        }
    };
    ch(o.listening != n.listening, "Listening");
    ch(o.fn_lock != n.fn_lock, "FnLock");
    ch(o.last_sleep_ts != n.last_sleep_ts, "LastSleepEvent");
    ch(o.wake_count != n.wake_count, "WakeCount");
    ch(o.eject_pressed != n.eject_pressed, "EjectPressed");
    ch(o.fn_pressed != n.fn_pressed, "FnPressed");
    ch(o.media_bits != n.media_bits, "MediaKeys");
    ch(o.batt_stat != n.batt_stat, "BatteryStatus");
    ch(o.powered_off != n.powered_off, "PoweredOff");
    if o != n {
        v.push("State");
    }
    v
}

/// Publishes the state of one process on the bus.
struct Publisher {
    conn: Connection,
    watch: Arc<Watch>,
    state: Shared,
    exported: HashSet<String>,
}

impl Publisher {
    /// MAC of the keyboard the state belongs to (snapshot, else unknown).
    fn mac(&self) -> Option<String> {
        self.watch.get().mac().map(str::to_ascii_uppercase)
    }

    /// Export the interface on the keyboard's object (idempotent).
    fn ensure(&mut self) -> Option<zbus::zvariant::OwnedObjectPath> {
        let path = device_path(&self.mac()?)?;
        if self.exported.insert(path.to_string()) {
            let r = self.conn.object_server().at(
                &path,
                Input {
                    state: self.state.clone(),
                },
            );
            if let Err(e) = r {
                tracing::warn!("cannot export Input on {path}: {e}");
                self.exported.remove(path.as_str());
                return None;
            }
            tracing::info!("D-Bus Input interface on {path}");
        }
        Some(path)
    }

    /// Notify the keyboard-driven alert unless the user already has the same
    /// alert by percentage ([`alerts::AlertDedupe`]); the keyboard flashes its
    /// CapsLock LED when critical, like the percentage alert does.
    fn keyboard_alert(&self, st: akm_core::registry::BatteryState, now: u64) {
        use akm_core::registry::BatteryState as B;
        let rank = if st == B::Critical { 2 } else { 1 };
        if !alerts::dedupe().allow_keyboard(rank, now) {
            tracing::info!(
                "keyboard {} alert not shown: the same alert was already raised",
                st.as_str()
            );
            return;
        }
        crate::notify::battery_state(st);
        // Apple's breaker blocks every emission, the LED included (R3, #251).
        if st == B::Critical && !akm_core::read_policy::tripped() {
            akm_core::led::flash_capslock_for(self.mac(), 5);
        }
    }

    fn handle(&mut self, msg: Msg) {
        let old = lock(&self.state);
        let mut sig: Option<PassiveEvent> = None;
        let now = now_unix();
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            match msg {
                Msg::Connected(_) => {
                    akm_core::link::clear_keyboard_off();
                    s.connected()
                }
                Msg::Disconnected(_) => s.disconnected(),
                Msg::Event(ev) => {
                    s.apply(ev, now);
                    sig = Some(ev);
                }
            }
        }
        let new = lock(&self.state);
        // The keyboard says its battery is low / critical (#189).
        let mut battery_alert: Option<akm_core::registry::BatteryState> = None;
        if let Some(PassiveEvent::BattStat { value }) = sig {
            battery_alert = passive::battery_state_alert(old.batt_stat, value);
            if akm_core::registry::BatteryState::from_byte(value)
                == akm_core::registry::BatteryState::Normal
            {
                // the keyboard is back to normal: percentage alerts are armed again
                alerts::dedupe().reset();
            }
            if let Some(st) = battery_alert {
                tracing::info!("keyboard reports battery state {}", st.as_str());
                if KEYBOARD_ALERTS.load(std::sync::atomic::Ordering::Relaxed) {
                    self.keyboard_alert(st, now);
                }
            }
        }
        if matches!(sig, Some(PassiveEvent::KeyboardOff)) {
            tracing::info!(
                "the keyboard announces that it switches off (0x13 bit 1 = 0): not a lost link"
            );
        }
        let Some(path) = self.ensure() else { return };
        let Ok(iref) = self.conn.object_server().interface::<_, Input>(&path) else {
            return;
        };
        let ctx = iref.signal_context();
        let d = iref.get();
        let props = changed_props(&old, &new);
        let r: zbus::Result<()> = zbus::block_on(async {
            for p in &props {
                match *p {
                    "Listening" => d.listening_changed(ctx).await?,
                    "FnLock" => d.fn_lock_changed(ctx).await?,
                    "LastSleepEvent" => d.last_sleep_event_changed(ctx).await?,
                    "WakeCount" => d.wake_count_changed(ctx).await?,
                    "EjectPressed" => d.eject_pressed_changed(ctx).await?,
                    "FnPressed" => d.fn_pressed_changed(ctx).await?,
                    "MediaKeys" => d.media_keys_changed(ctx).await?,
                    "BatteryStatus" => d.battery_status_changed(ctx).await?,
                    "PoweredOff" => d.powered_off_changed(ctx).await?,
                    "State" => d.state_changed(ctx).await?,
                    _ => {}
                }
            }
            if let Some(st) = battery_alert {
                Input::battery_alert(ctx, st.as_str()).await?;
            }
            match sig {
                Some(PassiveEvent::KeyboardOff) => {
                    Input::keyboard_off(ctx, new.last_off_ts).await?
                }
                Some(PassiveEvent::Sleep { code }) => {
                    Input::sleep_event(ctx, new.last_sleep_ts, code).await?
                }
                Some(PassiveEvent::Wake {
                    ready,
                    conn_request,
                }) => Input::wake(ctx, ready, conn_request, new.wake_count).await?,
                Some(PassiveEvent::Keys { eject, .. }) if eject != old.eject_pressed => {
                    Input::eject_changed(ctx, eject).await?
                }
                Some(PassiveEvent::FnLock { value }) if new.fn_lock != old.fn_lock => {
                    Input::fn_lock_updated(ctx, i32::from(value)).await?
                }
                _ => {}
            }
            Ok(())
        });
        if let Err(e) = r {
            tracing::warn!("cannot emit Input signals: {e}");
        }
    }
}

/// Stops the listener when dropped.
pub struct PassiveHandle {
    _listener: passive::Handle,
}

/// Start listening on the node `find` returns and publish on `conn`.
/// Two threads: the listener (blocking reads) and a publisher fed by a
/// channel, so neither the acquisition thread nor the hidraw reads wait for
/// the bus.
pub fn start(
    conn: Connection,
    watch: Arc<Watch>,
    cfg: Config,
    find: impl Fn() -> Option<PathBuf> + Send + 'static,
) -> std::io::Result<PassiveHandle> {
    let (tx, rx): (_, Receiver<Msg>) = mpsc::channel();
    // The acquisition thread feeds the read `0x30` through the same channel.
    let _ = INJECT.set(Mutex::new(tx.clone()));
    let state: Shared = Arc::new(Mutex::new(PassiveState::default()));
    let mut publisher = Publisher {
        conn,
        watch,
        state,
        exported: HashSet::new(),
    };
    std::thread::Builder::new()
        .name("kb-passive-dbus".into())
        .spawn(move || {
            // Ends when every sender (listener, `inject`) is dropped.
            for msg in rx {
                publisher.handle(msg);
            }
        })?;
    let listener = passive::spawn(cfg, find, move |m| {
        let _ = tx.send(m);
    })?;
    LISTENING.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(PassiveHandle {
        _listener: listener,
    })
}

/// [`start`] on the real keyboard node (`/dev/hidrawN` of the Apple keyboard).
pub fn start_default(conn: Connection, watch: Arc<Watch>) -> std::io::Result<PassiveHandle> {
    start(conn, watch, Config::default(), || {
        akm_core::hidraw::find_apple_hidraw().map(PathBuf::from)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_props_lists_only_differences() {
        let a = PassiveState::default();
        assert!(changed_props(&a, &a).is_empty());
        let mut b = a.clone();
        b.apply(PassiveEvent::FnLock { value: 2 }, 5);
        assert_eq!(changed_props(&a, &b), vec!["FnLock", "State"]);
        let mut c = b.clone();
        c.apply(
            PassiveEvent::Wake {
                ready: true,
                conn_request: false,
            },
            9,
        );
        assert_eq!(changed_props(&b, &c), vec!["WakeCount", "State"]);
        c.disconnected();
        assert!(changed_props(&b, &c).contains(&"FnLock"));
    }
}
