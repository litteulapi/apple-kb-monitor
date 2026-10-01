//! `apple-kb-monitord` — headless owner of the Apple keyboard
//! (docs/REVUE-ARCHITECTURE-GLOBALE.md §4, issues #61/#62).
//!
//! * [`actor`]   the single acquisition thread (hidraw, power_supply, RSSI,
//!   BlueZ provider, history, low-battery notification)
//! * [`watcher`] BlueZ / UPower signal watcher (system bus)
//! * [`bluez`]   `org.bluez.BatteryProvider1`
//! * [`service`] session interface `com.agenceapi.AppleKbMonitor1`
//! * [`client`]  D-Bus client + direct one-shot read (daemon absent)
//! * [`notify`]  desktop notifications (zbus, no notify-rust)
//!
//! No GUI dependency: `apihub-app` (egui) is a client of this crate.

pub mod actor;
pub mod bluez;
pub mod client;
pub mod notify;
pub mod service;
pub mod watcher;
