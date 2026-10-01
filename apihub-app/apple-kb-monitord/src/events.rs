//! In-process event hub: what the actor detects (threshold crossings,
//! battery replacements, link changes) fanned out to consumers — the D-Bus
//! service (signals of the v2 device objects) and any other in-process
//! consumer (e.g. a tray) through [`EventHub::subscribe`], without touching
//! the [`akm_core::Watch`] contract.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use akm_core::alerts::Crossing;
use akm_core::batteries::Replacement;
use akm_core::link::LinkEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceEvent {
    /// A low-battery threshold was crossed downwards (#82).
    BatteryLevelCrossed { mac: String, crossing: Crossing },
    /// New batteries detected (#85).
    BatteryReplaced {
        mac: String,
        replacement: Replacement,
    },
    /// Disconnection / reconnection as notified to the user (#84); a
    /// switch-off announced by the keyboard is `LinkEvent::PoweredOff` (#190).
    Link(LinkEvent),
}

impl DeviceEvent {
    pub fn mac(&self) -> &str {
        match self {
            DeviceEvent::BatteryLevelCrossed { mac, .. }
            | DeviceEvent::BatteryReplaced { mac, .. } => mac,
            DeviceEvent::Link(LinkEvent::Disconnected { mac })
            | DeviceEvent::Link(LinkEvent::PoweredOff { mac })
            | DeviceEvent::Link(LinkEvent::Reconnected { mac, .. }) => mac,
        }
    }
}

/// Fan-out of [`DeviceEvent`]s; dead subscribers are dropped on publish.
#[derive(Debug, Default)]
pub struct EventHub {
    subs: Mutex<Vec<Sender<DeviceEvent>>>,
}

impl EventHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn subscribe(&self) -> Receiver<DeviceEvent> {
        let (tx, rx) = mpsc::channel();
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
        rx
    }

    /// Deliver to every live subscriber; returns how many received it.
    pub fn publish(&self, ev: DeviceEvent) -> usize {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|tx| tx.send(ev.clone()).is_ok());
        subs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_out_and_dead_subscribers() {
        let hub = EventHub::new();
        assert_eq!(
            hub.publish(DeviceEvent::Link(LinkEvent::Disconnected {
                mac: "A".into()
            })),
            0
        );
        let a = hub.subscribe();
        let b = hub.subscribe();
        let ev = DeviceEvent::Link(LinkEvent::Reconnected {
            mac: "A".into(),
            pct: Some(50.0),
        });
        assert_eq!(hub.publish(ev.clone()), 2);
        assert_eq!(a.recv().unwrap(), ev);
        assert_eq!(b.recv().unwrap().mac(), "A");
        drop(b);
        assert_eq!(hub.publish(ev), 1);
    }
}
