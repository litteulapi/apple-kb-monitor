//! Telemetry data model of one keyboard (serialisable: it travels over D-Bus and in `--json`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbDevice {
    pub model: Option<String>,
    /// Name the kernel registered the HID device under (`HID_NAME`).
    pub name: Option<String>,
    /// User-chosen name on this computer (`BlueZ` `Device1.Alias`).
    pub alias: Option<String>,
    pub mac: Option<String>,
    pub chip: Option<String>,
    pub driver: Option<String>,
    /// Name stored IN the keyboard, distinct from `name` and `alias`.
    pub name_on_keyboard: Option<String>,
    /// The same 32 bytes as hex (backup source of `akmctl rename --device-name`).
    pub name_on_keyboard_hex: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbBattery {
    /// Kernel `power_supply` capacity, else report 0x47 (Battery Strength).
    pub percentage: Option<f64>,
    /// Compatibility mirror of `percentage` for D-Bus/QML clients that read this name.
    pub percentage_fine: Option<f64>,
    /// \[hypothesis\] Estimate at 0.1 % from the filtered voltage on the 0x5A curve of the unit.
    pub percentage_estimate: Option<f64>,
    /// Compatibility alias of `percentage_estimate` (former name, now 0.1 % instead of an integer).
    pub percentage_interpolated: Option<f64>,
    /// \[measured\] Cell voltage in volts = `voltage_mv / 1000`.
    pub voltage: Option<f64>,
    /// \[measured\] Cell voltage in mV.
    pub voltage_mv: Option<u32>,
    /// \[measured\] value, \[hypothesis\] "filtered": report 0x49, u16 LE, mV.
    pub voltage_filtered_mv: Option<u32>,
    /// 0x46 and 0xFF were both read and differ by more than 20 mV: the sample is doubtful.
    pub voltage_doubtful: bool,
    /// \[measured\] Report 0xF5, u16 big-endian, uninterpreted.
    pub adc_raw: Option<u32>,
    /// \[hypothesis\] Real charge estimated by the declared chemistry, filled by the daemon.
    pub charge_estimate: Option<crate::chemistry::ChargeEstimate>,
    /// Batteries installed less than two days ago: no figure is drawn from the voltage.
    pub new_batteries: bool,
    /// Battery thresholds Full / Low / Critical / Empty read once per connection from report 0x60
    /// \[plist\].
    pub thresholds: Option<crate::registry::Thresholds>,
    /// Where the filtered voltage (else the instantaneous one) sits against [`Self::thresholds`].
    pub threshold_level: Option<String>,
    /// The percentage as macOS shows it ("Apple display"): `IOBluetooth` remaps raw 0x47.
    pub apple_display_pct: Option<f64>,
    /// Margins in mV above Full / Low / Critical / Empty (negative = below).
    pub threshold_margins_mv: Option<[i32; 4]>,
    /// \[disassembly\] Battery state read by GET Input `0x30` right after `0x47` (Apple's
    /// `getBatteryState`, R2): 0 normal, 1 low, 2-3 critical.
    pub state: Option<u8>,
    /// The values above were not read by the last acquisition.
    pub kept: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbBluetooth {
    pub connected: bool,
    pub paired: bool,
    pub rssi_dbus: Option<i32>,
    pub tx_power_dbus: Option<i32>,
    /// \[measured\] Report 0x4C bytes 2..8 reversed.
    pub paired_host_addr: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbRadio {
    /// **Deprecated name, kept for compatibility**.
    pub rssi_dbm: Option<i32>,
    /// Relative RSSI in dB, 0 = ideal range, negative = below, positive = above (legal).
    pub rssi_rel_db: Option<i32>,
    /// `"bredr-golden-range"` for the classic link of this keyboard.
    pub rssi_kind: Option<String>,
    /// `"excellent"`, `"good"` or `"weak"`.
    pub rssi_quality: Option<String>,
    pub tx_power_dbm: Option<i32>,
    /// Why the signal is not measured while the keyboard is connected.
    pub rssi_error: Option<crate::rssi::RssiIssue>,
}

impl KbRadio {
    /// Sets the RSSI fields from one BR/EDR relative measurement.
    pub fn set_rssi_rel(&mut self, rel: Option<i32>) {
        let rel = crate::signal::valid_rel(rel);
        self.rssi_dbm = rel;
        self.rssi_rel_db = rel;
        self.rssi_kind = rel.map(|_| crate::signal::KIND_BREDR.to_string());
        self.rssi_quality = rel.map(|r| crate::signal::quality(r).as_str().to_string());
    }

    /// Relative RSSI whatever the writer: new field, else the old name.
    #[must_use]
    pub fn rel_db(&self) -> Option<i32> {
        crate::signal::valid_rel(self.rssi_rel_db.or(self.rssi_dbm))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbFirmware {
    /// Report 0x4F as u16 little-endian, shown `0x0050`.
    pub version: Option<String>,
    /// Same value as `version` under its explicit JSON name (`firmware.version_hex`).
    pub version_hex: Option<String>,
    /// Latest public version known for this product id (`0x0050`), from the embedded table
    /// [`crate::firmware::KNOWN_FIRMWARE`]; `None` = model not in the table.
    pub latest_known: Option<String>,
    /// `up_to_date` / `update_available` / `unknown` (empty = never assessed).
    pub status: String,
    /// Where the table entry comes from.
    pub source: Option<String>,
    pub table_date: Option<String>,
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
    /// Uninterpreted vendor reports, `"0xNN"` -> payload hex (report id excluded).
    pub raw: BTreeMap<String, String>,
    /// The daemon's circuit breaker was open when this report was built.
    pub breaker_open: bool,
    /// The vendor read stopped early: the fields above may be partial.
    pub incomplete: bool,
}

impl KbReport {
    /// Best battery percentage: kernel/0x47 > compatibility mirror > interpolated estimate.
    #[must_use]
    pub fn battery_pct(&self) -> Option<f64> {
        let ok = |p: Option<f64>| p.filter(|v| v.is_finite() && (0.0..=100.0).contains(v));
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
    fn rssi_fields_are_set_together_and_old_json_still_reads() {
        let mut r = KbRadio::default();
        r.set_rssi_rel(Some(0));
        assert_eq!(r.rssi_rel_db, Some(0));
        assert_eq!(r.rssi_dbm, Some(0), "compat mirror");
        assert_eq!(r.rssi_kind.as_deref(), Some("bredr-golden-range"));
        assert_eq!(r.rssi_quality.as_deref(), Some("excellent"));
        r.set_rssi_rel(Some(-7));
        assert_eq!(r.rssi_quality.as_deref(), Some("weak"));
        r.set_rssi_rel(Some(127));
        assert_eq!(
            (r.rssi_dbm, r.rssi_rel_db, r.rssi_quality.clone()),
            (None, None, None)
        );
        let old: KbReport = serde_json::from_str(r#"{"radio":{"rssi_dbm":-2}}"#).unwrap();
        assert_eq!(old.radio.rel_db(), Some(-2));
        assert_eq!(old.radio.rssi_quality, None);
    }

    #[test]
    fn battery_pct_out_of_range_falls_through() {
        let mut r = KbReport::default();
        r.battery.percentage_fine = Some(255.0);
        assert_eq!(r.battery_pct(), None);
        r.battery.percentage_interpolated = Some(60.0);
        assert_eq!(r.battery_pct(), Some(60.0));
    }

    #[test]
    fn battery_pct_priority_and_nan() {
        let mut r = KbReport::default();
        assert_eq!(r.battery_pct(), None);
        r.battery.percentage_estimate = Some(60.0);
        assert_eq!(r.battery_pct(), Some(60.0));
        r.battery.percentage_fine = Some(90.0);
        assert_eq!(r.battery_pct(), Some(90.0));
        r.battery.percentage = Some(50.0);
        assert_eq!(r.battery_pct(), Some(50.0));
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
        r.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        r.battery.voltage = Some(2.81);
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<KbReport>(&s).unwrap(), r);
        let partial: KbReport =
            serde_json::from_str(r#"{"battery":{"percentage":90},"x":1}"#).unwrap();
        assert_eq!(partial.battery.percentage, Some(90.0));
    }
}
