//! `akm-core` — everything about the Apple Bluetooth keyboard that does not need a GUI nor a D-Bus
//! connection (docs/ARCHITECTURE.md).

pub mod activity;
pub mod advice;
pub mod alerts;
pub mod alias;
pub mod apple_model;
pub mod batteries;
pub mod breaker_state;
pub mod calibration;
pub mod chemistry;
pub mod config;
pub mod conv;
pub mod decode;
pub mod deferred;
pub mod device_settings;
pub mod devname;
#[cfg(test)]
mod discover;
pub mod firmware;
pub mod forecast;
pub mod fsutil;
pub mod hid_params;
pub mod hidraw;
pub mod history;
pub mod history_limits;
pub mod i18n;
pub mod keycodes;
pub mod keymap;
mod keymap_file;
pub mod keytable;
pub mod led;
pub mod link;
pub mod linkstats;
pub mod machine;
pub mod model;
pub mod parity;
pub mod passive;
pub mod paths;
pub mod power;
pub mod quiet;
pub mod read_policy;
pub mod recovery;
pub mod registry;
pub mod reminder;
pub mod report;
pub mod roster;
pub mod rssi;
pub mod signal;
pub mod snapshot;
#[cfg(any(test, feature = "srclint"))]
pub mod srclint;
pub mod stale;
pub mod usage;

/// Full version of the build.
pub const PKG_VERSION: &str = match option_env!("AKM_PKG_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub use report::KbReport;
pub use snapshot::{Snapshot, Watch};
