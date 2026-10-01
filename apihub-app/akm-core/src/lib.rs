//! `akm-core` — everything about the Apple Bluetooth keyboard that does not
//! need a GUI nor a D-Bus connection (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.1).
//!
//! * [`model`]       supported models, `HID_ID` / `HID_UNIQ` parsing
//! * [`report`]      telemetry data model ([`report::KbReport`])
//! * [`chemistry`]   charge estimate by declared battery chemistry (#178)
//! * [`calibration`] ADC → voltage → percentage, battery chemistry
//! * [`decode`]      [`decode::HidSource`] trait, [`decode::Fixture`], decoding of
//!   the vendor Feature Reports (testable without hardware)
//! * [`discover`]    keyboard discovery / selection (model table + HID descriptor)
//! * [`hidraw`]      the real `/dev/hidrawN` source + wake monitor
//! * [`power`]       kernel `power_supply` battery (source of truth for the %)
//! * [`history`]     JSONL history, injectable clock, rotation, estimate
//! * [`machine`]     pure connection/scheduling state machine
//! * [`rssi`]        RSSI helper runner + freshness tracker
//! * [`led`]         keyboard LED state (sysfs) and control
//! * [`signal`]     link quality from the relative BR/EDR RSSI (#174)
//! * [`snapshot`]    immutable published state + `Watch` (Arc + version)
//! * [`alerts`]      multi-threshold low-battery alerts with hysteresis (#82)
//! * [`forecast`]    autonomy forecast in days from the history (#83)
//! * [`batteries`]   battery replacement detection, per-set lifetime (#85)
//! * [`link`]        connection / reconnection notification logic (#84)
//! * [`config`]      `config.toml` (small TOML subset, no dependency)
//! * [`hid_params`]  `hid_apple` parameters: read + whitelist
//! * [`keymap`]      special keys: kernel Fn table, effective table, manual
//!   mapping (`keymap.toml` → udev hwdb), xkb/KDE names (#247)
//! * [`keycodes`]    `KEY_*` names of `input-event-codes.h`
//! * [`passive`]     passive listening to input reports 0x04/0x05/0x30/0x13/0x11/0x12
//!   (#130, #187, #129)
//! * [`registry`]    register map of every known HID report + safety classes (#219)
//! * [`firmware`]    firmware version check against the embedded table (#219)
//! * [`alias`]       validation of the user-chosen keyboard name (#141)

pub mod alerts;
pub mod alias;
pub mod batteries;
pub mod calibration;
pub mod chemistry;
pub mod config;
pub mod decode;
pub mod discover;
pub mod firmware;
pub mod forecast;
pub mod hid_params;
pub mod keycodes;
pub mod keymap;
pub mod hidraw;
pub mod history;
pub mod led;
pub mod link;
pub mod machine;
pub mod model;
pub mod parity;
pub mod passive;
pub mod power;
pub mod read_policy;
pub mod recovery;
pub mod registry;
pub mod report;
pub mod rssi;
pub mod signal;
pub mod snapshot;

pub use report::KbReport;
pub use snapshot::{Snapshot, Watch};
