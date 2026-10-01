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
//! * [`devices`] D-Bus API v2: one object per keyboard (#93)
//! * [`events`]  in-process event hub (alerts, replacements, link changes)
//! * [`passive`]  passive input-report listening, `...Input` interface (#130, #187, #129)
//! * [`repair`]  link keeper: health, reconnection, repair launcher (#144)
//! * [`sleep`]   logind sleep / resume (#145) and shutdown hook
//! * [`shutdown`] `WillShutdown` (Feature `0x40`) at shutdown, as macOS does (#191)
//! * [`settings`] `hid_apple` write path through the privileged helper
//! * [`alias`]   keyboard name on this computer (BlueZ `Alias`, #141)
//!
//! No GUI dependency: `apihub-app` (egui) is a client of this crate.

pub mod actor;
pub mod alias;
pub mod bluez;
pub mod client;
pub mod devices;
pub mod events;
pub mod keymap;
pub mod notify;
pub mod passive;
pub mod repair;
pub mod service;
pub mod settings;
pub mod shutdown;
pub mod sleep;
pub mod watcher;
