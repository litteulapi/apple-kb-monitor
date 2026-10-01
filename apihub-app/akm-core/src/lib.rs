//! `akm-core` — everything about the Apple Bluetooth keyboard that does not
//! need a GUI nor a D-Bus connection (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.1).
//!
//! * [`model`]       supported models, `HID_ID` / `HID_UNIQ` parsing
//! * [`report`]      telemetry data model ([`report::KbReport`])
//! * [`calibration`] ADC → voltage → percentage, battery chemistry
//! * [`decode`]      [`decode::HidSource`] trait, [`decode::Fixture`], decoding of
//!   the vendor Feature Reports (testable without hardware)
//! * [`hidraw`]      the real `/dev/hidrawN` source + wake monitor
//! * [`power`]       kernel `power_supply` battery (source of truth for the %)
//! * [`history`]     JSONL history, injectable clock, rotation, estimate
//! * [`machine`]     pure connection/scheduling state machine
//! * [`rssi`]        RSSI helper runner + freshness tracker
//! * [`led`]         keyboard LED state (sysfs) and control
//! * [`snapshot`]    immutable published state + `Watch` (Arc + version)

pub mod calibration;
pub mod decode;
pub mod hidraw;
pub mod history;
pub mod led;
pub mod machine;
pub mod model;
pub mod power;
pub mod report;
pub mod rssi;
pub mod snapshot;

pub use report::KbReport;
pub use snapshot::{Snapshot, Watch};
