//! Telemetry data model of one keyboard (serialisable: it travels over D-Bus
//! and in `--json`).

use std::collections::BTreeMap;

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
    /// Kernel `power_supply` capacity, else report 0x47 (Battery Strength).
    /// [mesuré] 0x47 = kernel capacity (99 = 99 on the A1314 ISO, 2026-10-01);
    /// [source] `hid-input.c` reads 0x47 as a 0..100 Feature report.
    pub percentage: Option<f64>,
    /// Compatibility mirror of `percentage` for D-Bus/QML clients that read
    /// this name. It is **never** report 0xEA any more: the "pre-rounding
    /// value" claim was refuted (0xEA = 98 while 0x47 = kernel = 99, #136).
    /// 0xEA is kept uninterpreted in [`KbReport::raw`].
    pub percentage_fine: Option<f64>,
    /// [hypothèse] Estimate at 0.1 % from the filtered voltage (0x49, else
    /// 0x46) on the 0x5A curve of the unit. Only an **estimate**: shown only
    /// when no kernel/0x47 percentage exists.
    pub percentage_estimate: Option<f64>,
    /// Compatibility alias of `percentage_estimate` (former name, now 0.1 %
    /// instead of an integer).
    pub percentage_interpolated: Option<f64>,
    /// [mesuré] Cell voltage in volts = `voltage_mv / 1000` (report 0x46, else
    /// 0xFF bytes 1-2). Never derived from 0xF5 (#139).
    pub voltage: Option<f64>,
    /// [mesuré] Cell voltage in mV: report 0x46 as u16 little-endian, fallback
    /// report 0xFF bytes 1-2 as u16 big-endian.
    pub voltage_mv: Option<u32>,
    /// [mesuré] value, [hypothèse] "filtered": report 0x49, u16 LE, mV.
    pub voltage_filtered_mv: Option<u32>,
    /// 0x46 and 0xFF were both read and differ by more than 20 mV: the
    /// sample is doubtful (0x46 is kept).
    pub voltage_doubtful: bool,
    /// [mesuré] Report 0xF5, u16 big-endian, uninterpreted (constant across a
    /// battery change: not a voltage).
    pub adc_raw: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbBluetooth {
    pub connected: bool,
    pub paired: bool,
    pub rssi_dbus: Option<i32>,
    pub tx_power_dbus: Option<i32>,
    /// [mesuré] Report 0x4C bytes 2..8 reversed: Bluetooth address of the host
    /// adapter the keyboard is paired with (= `HID_PHYS`), e.g. `6C:94:66:52:7C:0D`.
    /// The 12 following bytes are unidentified and never published (#123, #133).
    pub paired_host_addr: Option<String>,
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
    /// Report 0x4F as u16 little-endian, shown `0x0050`. [mesuré] equals the
    /// DID version of the BlueZ modalias (`usb:v05ACp0256d0050`).
    pub version: Option<String>,
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
    /// Uninterpreted vendor reports, `"0xNN"` -> payload hex (report id
    /// excluded). Their meaning is not proven (docs/AUDIT-DECODAGE-HID.md):
    /// 0x46, 0x49 and 0xFF are **not** LE link parameters nor a build number
    /// (#131, #132). 0x4C is never included (#123).
    pub raw: BTreeMap<String, String>,
    /// The vendor read stopped early (request timed out, device gone or time
    /// budget spent, #134): the fields above may be partial.
    pub incomplete: bool,
}

impl KbReport {
    /// Best battery percentage: kernel/0x47 (`percentage`) > compatibility
    /// mirror > interpolated estimate. Every term is filtered on its own, so a
    /// NaN falls through to the next one. 0xEA is not a percentage source.
    pub fn battery_pct(&self) -> Option<f64> {
        let ok = |p: Option<f64>| p.filter(|v| v.is_finite());
        ok(self.battery.percentage)
            .or(ok(self.battery.percentage_fine))
            .or(ok(self.battery.percentage_estimate))
            .or(ok(self.battery.percentage_interpolated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_pct_priority_and_nan() {
        let mut r = KbReport::default();
        assert_eq!(r.battery_pct(), None);
        r.battery.percentage_estimate = Some(60.0);
        assert_eq!(r.battery_pct(), Some(60.0));
        r.battery.percentage_fine = Some(90.0);
        assert_eq!(r.battery_pct(), Some(90.0));
        // Kernel / 0x47 wins over everything (#136).
        r.battery.percentage = Some(50.0);
        assert_eq!(r.battery_pct(), Some(50.0));
        // A NaN falls through to the next finite term.
        r.battery.percentage = Some(f64::NAN);
        assert_eq!(r.battery_pct(), Some(90.0));
        r.battery.percentage_fine = Some(f64::INFINITY);
        assert_eq!(r.battery_pct(), Some(60.0));
        r.battery.percentage_estimate = None;
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
