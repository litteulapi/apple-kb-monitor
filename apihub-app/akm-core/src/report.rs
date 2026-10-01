//! Telemetry data model of one keyboard (serialisable: it travels over D-Bus
//! and in `--json`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbDevice {
    pub model: Option<String>,
    /// Name the kernel registered the HID device under (`HID_NAME`).
    pub name: Option<String>,
    /// User-chosen name on this computer (BlueZ `Device1.Alias`, #141).
    pub alias: Option<String>,
    pub mac: Option<String>,
    pub chip: Option<String>,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbBattery {
    /// Kernel `power_supply` capacity, else firmware-rounded report 0x47.
    pub percentage: Option<f64>,
    /// Precise value (report 0xEA), replaced by the kernel value when known.
    pub percentage_fine: Option<f64>,
    /// Interpolated from voltage + calibration curve (diagnostic).
    pub percentage_interpolated: Option<f64>,
    pub voltage: Option<f64>,
    pub adc_raw: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbBluetooth {
    pub connected: bool,
    pub paired: bool,
    pub rssi_dbus: Option<i32>,
    pub tx_power_dbus: Option<i32>,
    pub conn_interval_ms: Option<f64>,
    pub slave_latency: Option<u8>,
    pub supervision_timeout_s: Option<f64>,
    pub identity_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbRadio {
    pub rssi_dbm: Option<i32>,
    pub tx_power_dbm: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbFirmware {
    pub version: Option<String>,
    pub build: Option<u32>,
    pub adc_ref: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbWake {
    /// Seconds since the last wake event (input report 0x13), if any was seen.
    pub last_age_s: Option<f64>,
    /// Number of wake events since the process started.
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbReport {
    pub wake: KbWake,
    pub device: KbDevice,
    pub battery: KbBattery,
    pub bluetooth: KbBluetooth,
    pub radio: KbRadio,
    pub firmware: KbFirmware,
}

impl KbReport {
    /// Best battery percentage available (kernel > precise > interpolated > rounded).
    pub fn battery_pct(&self) -> Option<f64> {
        self.battery
            .percentage_fine
            .or(self.battery.percentage_interpolated)
            .or(self.battery.percentage)
            .filter(|p| p.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_pct_priority_and_nan() {
        let mut r = KbReport::default();
        assert_eq!(r.battery_pct(), None);
        r.battery.percentage = Some(50.0);
        assert_eq!(r.battery_pct(), Some(50.0));
        r.battery.percentage_interpolated = Some(60.0);
        assert_eq!(r.battery_pct(), Some(60.0));
        r.battery.percentage_fine = Some(90.0);
        assert_eq!(r.battery_pct(), Some(90.0));
        r.battery.percentage_fine = Some(f64::NAN);
        r.battery.percentage_interpolated = None;
        r.battery.percentage = None;
        assert_eq!(r.battery_pct(), None);
    }

    #[test]
    fn json_roundtrip_and_tolerant_decoding() {
        let mut r = KbReport::default();
        r.device.mac = Some("04:DB:56:CA:42:EE".into());
        r.battery.voltage = Some(2.81);
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<KbReport>(&s).unwrap(), r);
        // Missing sections / unknown fields are accepted (forward compatible).
        let partial: KbReport =
            serde_json::from_str(r#"{"battery":{"percentage":90},"x":1}"#).unwrap();
        assert_eq!(partial.battery.percentage, Some(90.0));
    }
}
