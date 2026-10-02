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
    /// Name stored IN the keyboard (`0x51`-`0x54`, read once per connection
    /// by the daemon), distinct from `name` and `alias` (#248).
    pub name_on_keyboard: Option<String>,
    /// The same 32 bytes as hex (backup source of `akmctl rename --device-name`).
    pub name_on_keyboard_hex: Option<String>,
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
    /// [hypothèse] Real charge estimated by the declared chemistry (#178),
    /// filled by the daemon. `percentage` stays the keyboard's own indication.
    pub charge_estimate: Option<crate::chemistry::ChargeEstimate>,
    /// Batteries installed less than two days ago: no figure is drawn from
    /// the voltage (#178).
    pub new_batteries: bool,
    /// Battery thresholds Full / Low / Critical / Empty read once per
    /// connection from report 0x60 [plist] (#215).
    pub thresholds: Option<crate::registry::Thresholds>,
    /// Where the filtered voltage (else the instantaneous one) sits against
    /// [`Self::thresholds`]: `ok` / `low` / `critical` / `empty`.
    pub threshold_level: Option<String>,
    /// The percentage as macOS shows it ("Apple display", #213): IOBluetooth
    /// remaps raw 0x47. Informative only: alerts and the estimate do not use it.
    pub apple_display_pct: Option<f64>,
    /// Margins in mV above Full / Low / Critical / Empty (negative = below).
    pub threshold_margins_mv: Option<[i32; 4]>,
    /// [désassemblage] Battery state read by GET Input `0x30` right after
    /// `0x47` (Apple's `getBatteryState`, R2 #251): 0 normal, 1 low, 2-3
    /// critical. `None` when not read in this burst.
    pub state: Option<u8>,
    /// The values above were not read by the last acquisition: they are the
    /// ones of an earlier read, kept so that the level does not turn "n/a"
    /// (#264). Never a new sample: `last_update` keeps the age of the read.
    pub kept: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbBluetooth {
    pub connected: bool,
    pub paired: bool,
    pub rssi_dbus: Option<i32>,
    pub tx_power_dbus: Option<i32>,
    /// [mesuré] Report 0x4C bytes 2..8 reversed: Bluetooth address of the host
    /// adapter the keyboard is paired with (= `HID_PHYS`), e.g. `AA:BB:CC:DD:EE:F2`.
    /// The 12 following bytes are unidentified and never published (#123, #133).
    pub paired_host_addr: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbRadio {
    /// **Deprecated name, kept for compatibility** (#174): on the BR/EDR link
    /// of this keyboard this is NOT a power in dBm but the gap in dB to the
    /// controller's ideal reception range (0 = ideal), same value as
    /// `rssi_rel_db`. Prefer `rssi_rel_db` and `rssi_quality`.
    pub rssi_dbm: Option<i32>,
    /// Relative RSSI in dB, 0 = ideal range, negative = below, positive =
    /// above (legal). Meaning given by `rssi_kind`.
    pub rssi_rel_db: Option<i32>,
    /// `"bredr-golden-range"` for the classic link of this keyboard.
    pub rssi_kind: Option<String>,
    /// `"excellent"`, `"good"` or `"weak"`.
    pub rssi_quality: Option<String>,
    pub tx_power_dbm: Option<i32>,
}

impl KbRadio {
    /// Sets the RSSI fields from one BR/EDR relative measurement (all of them,
    /// or all cleared when the value is unknown).
    pub fn set_rssi_rel(&mut self, rel: Option<i32>) {
        let rel = crate::signal::valid_rel(rel);
        self.rssi_dbm = rel;
        self.rssi_rel_db = rel;
        self.rssi_kind = rel.map(|_| crate::signal::KIND_BREDR.to_string());
        self.rssi_quality = rel.map(|r| crate::signal::quality(r).as_str().to_string());
    }

    /// Relative RSSI whatever the writer: new field, else the old name.
    pub fn rel_db(&self) -> Option<i32> {
        crate::signal::valid_rel(self.rssi_rel_db.or(self.rssi_dbm))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KbFirmware {
    /// Report 0x4F as u16 little-endian, shown `0x0050`. [mesuré] equals the
    /// DID version of the BlueZ modalias (`usb:v05ACp0256d0050`).
    pub version: Option<String>,
    /// Same value as `version` under its explicit JSON name (`firmware.version_hex`, #219).
    pub version_hex: Option<String>,
    /// Latest public version known for this product id (`0x0050`), from the
    /// embedded table [`crate::firmware::KNOWN_FIRMWARE`]; `None` = model not in the table.
    pub latest_known: Option<String>,
    /// `up_to_date` / `update_available` / `unknown` (empty = never assessed).
    pub status: String,
    /// Where the table entry comes from.
    pub source: Option<String>,
    /// Date of the embedded table.
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
    /// Uninterpreted vendor reports, `"0xNN"` -> payload hex (report id
    /// excluded). Their meaning is not proven (docs/AUDIT-DECODAGE-HID.md):
    /// 0x46, 0x49 and 0xFF are **not** LE link parameters nor a build number
    /// (#131, #132). 0x4C is never included (#123).
    pub raw: BTreeMap<String, String>,
    /// The daemon's circuit breaker was open when this report was built
    /// (pre-flight of the writes, #248).
    pub breaker_open: bool,
    /// The vendor read stopped early (request timed out, device gone or time
    /// budget spent, #134): the fields above may be partial.
    pub incomplete: bool,
}

impl KbReport {
    /// Best battery percentage: kernel/0x47 (`percentage`) > compatibility
    /// mirror > interpolated estimate. Every term is filtered on its own, so a
    /// NaN falls through to the next one. 0xEA is not a percentage source.
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
        assert_eq!((r.rssi_dbm, r.rssi_rel_db, r.rssi_quality.clone()), (None, None, None));
        // JSON written before #174 only has rssi_dbm.
        let old: KbReport = serde_json::from_str(r#"{"radio":{"rssi_dbm":-2}}"#).unwrap();
        assert_eq!(old.radio.rel_db(), Some(-2));
        assert_eq!(old.radio.rssi_quality, None);
    }

    #[test]
    fn battery_pct_out_of_range_falls_through() {
        // #163: raw probe byte 255 is not a percentage.
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
        r.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        r.battery.voltage = Some(2.81);
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<KbReport>(&s).unwrap(), r);
        // Missing sections / unknown fields are accepted (forward compatible).
        let partial: KbReport =
            serde_json::from_str(r#"{"battery":{"percentage":90},"x":1}"#).unwrap();
        assert_eq!(partial.battery.percentage, Some(90.0));
    }
}
