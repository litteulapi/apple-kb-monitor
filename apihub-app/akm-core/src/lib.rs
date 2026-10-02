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
//! * [`history_limits`] size bound of the history file (5 MiB) and point bound of
//!   what a client reads (#96)
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
//! * [`keytable`]    effective table JSON, keyboard detection, apply plan
//! * [`keycodes`]    `KEY_*` names of `input-event-codes.h`
//! * [`passive`]     passive listening to input reports 0x04/0x05/0x30/0x13/0x11/0x12
//!   (#130, #187, #129)
//! * [`registry`]    register map of every known HID report + safety classes (#219)
//! * [`firmware`]    firmware version check against the embedded table (#219)
//! * [`alias`]       validation of the user-chosen keyboard name (#141)
//! * [`devname`]     name stored IN the keyboard (0x51-0x55): validation, frames,
//!   backup, guarded write behind three locks (#248)
//! * [`reminder`]    when "change the batteries" (keyboard thresholds 0x60) and
//!   "firmware update" notices are due, once per crossing
//! * [`breaker_state`] the daemon's circuit breaker published in
//!   `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state` for the other emitters
//!   (`akm-hid-control`, `akmctl`; #244, #251)

pub mod alerts;
pub mod alias;
pub mod apple_model;
pub mod batteries;
pub mod breaker_state;
pub mod calibration;
pub mod chemistry;
pub mod config;
pub mod decode;
pub mod devname;
pub mod discover;
pub mod firmware;
pub mod forecast;
pub mod hid_params;
pub mod keycodes;
pub mod keymap;
pub mod keytable;
pub mod hidraw;
pub mod history;
pub mod history_limits;
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
pub mod reminder;
pub mod report;
pub mod rssi;
pub mod signal;
pub mod snapshot;

pub use report::KbReport;
pub use snapshot::{Snapshot, Watch};
