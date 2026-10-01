//! `dump`: SAFE version. Reads only the routine reports of the read policy
//! (`akm_core::read_policy::routine_reads()`: Feature 0x47, then GET Input
//! 0x30 as Apple does (#251), then 0x46, 0x49), through `read_safe`
//! (allow-list, spacing, time budget, stop at the first failure).
//! No scan, never 0x4C / 0xFE / 0x01. The reverse-engineering scripts live in
//! `tests/live/`, not here.

use akm_core::decode::HidSource;
use akm_core::read_policy::{self, SafeRead};
use akm_core::report::KbReport;
use serde_json::{json, Value};

pub struct Dump {
    pub outcome: SafeRead,
    pub report: KbReport,
}

/// Read the allowed reports from `src`.
pub fn collect(src: &dyn HidSource) -> Dump {
    let mut report = KbReport::default();
    let outcome = read_policy::read_safe(src, &mut report);
    Dump { outcome, report }
}

fn le_mv(hex: &str) -> Option<u32> {
    let b0 = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
    let b1 = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
    Some(u32::from(u16::from_le_bytes([b0, b1])))
}

fn meaning(id: &str, hex: &str) -> String {
    match id {
        "0x47" => u8::from_str_radix(hex.get(0..2).unwrap_or(""), 16)
            .map_or("?".into(), |p| format!("battery {p} %")),
        "0x46" => le_mv(hex).map_or("?".into(), |mv| format!("cell voltage {mv} mV (measured)")),
        "0x49" => le_mv(hex).map_or("?".into(), |mv| {
            format!("filtered voltage {mv} mV (measured)")
        }),
        "input 0x30" => {
            u8::from_str_radix(hex.get(0..2).unwrap_or(""), 16).map_or("?".into(), |b| {
                format!(
                    "battery state {} (GET Input, Apple R2)",
                    akm_core::registry::BatteryState::from_byte(b).as_str()
                )
            })
        }
        _ => "?".into(),
    }
}

fn outcome_str(o: SafeRead) -> &'static str {
    match o {
        SafeRead::Complete => "complete",
        SafeRead::Partial => "partial (stopped at the first failure)",
        SafeRead::Skipped(_) => "skipped",
    }
}

pub fn to_text(d: &Dump) -> String {
    let mut out = format!(
        "Safe read of reports {:?} + Input {:?}: {}\n",
        read_policy::ALLOWED,
        akm_core::registry::SAFE_READ_INPUT_IDS,
        outcome_str(d.outcome)
    );
    for id in ["0x47", "input 0x30", "0x46", "0x49"] {
        match d.report.raw.get(id) {
            Some(h) => out.push_str(&format!("  {id}  {h:<8}  {}\n", meaning(id, h))),
            None => out.push_str(&format!("  {id}  (not read)\n")),
        }
    }
    out
}

pub fn to_json(d: &Dump) -> Value {
    let reports: serde_json::Map<String, Value> = d
        .report
        .raw
        .iter()
        .map(|(k, h)| (k.clone(), json!({"hex": h, "meaning": meaning(k, h)})))
        .collect();
    json!({
        "schema": crate::status::SCHEMA,
        "outcome": outcome_str(d.outcome),
        "incomplete": d.report.incomplete,
        "reports": reports,
    })
}

/// Open the keyboard's hidraw node and read the routine reports.
/// Takes the cross-process read lock shared with the daemon; refuses a
/// keyboard that is not a BCM2042 (the only family with these reports).
pub fn run() -> Result<Dump, String> {
    use std::os::fd::AsRawFd;
    let dev = akm_core::hidraw::find_apple_hidraw().ok_or("no Apple keyboard found (hidraw)")?;
    let uevent = std::fs::read_to_string(format!("/sys/class/hidraw/{}/device/uevent", dev.trim_start_matches("/dev/")))
        .unwrap_or_default();
    if akm_core::model::family_from_uevent(&uevent) != akm_core::model::Family::Bcm2042 {
        return Err(format!("{dev} is not a BCM2042 keyboard: nothing to read"));
    }
    let _lock = read_policy::try_lock(read_policy::LOCK_WAIT)
        .ok_or("another reader holds the HID lock (the daemon is reading): retry in a moment")?;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&dev)
        .map_err(|e| format!("cannot open {dev}: {e}"))?;
    Ok(collect(&akm_core::hidraw::Hidraw(f.as_raw_fd())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;

    struct Spy<'a> {
        inner: &'a dyn HidSource,
        log: RefCell<Vec<u8>>,
    }
    impl HidSource for Spy<'_> {
        fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
            self.log.borrow_mut().push(id);
            self.inner.feature(id)
        }
        fn input(&self, id: u8) -> io::Result<Vec<u8>> {
            self.log.borrow_mut().push(id);
            self.inner.input(id)
        }
    }

    fn fixture() -> akm_core::decode::Fixture {
        akm_core::decode::Fixture::new()
            .with_input(&[0x30, 1])
            .with(&[0x47, 99])
            .with(&[0x46, 0xBA, 0x0B])
            .with(&[0x49, 0x89, 0x0B])
            .with(&[0xFE, 1, 2, 3])
            .with(&[0x4C, 9, 9])
            .with(&[0x01, 1])
    }

    #[test]
    fn only_the_three_allowed_reports_are_ever_requested() {
        let f = fixture();
        let spy = Spy {
            inner: &f,
            log: RefCell::new(vec![]),
        };
        let d = collect(&spy);
        assert_eq!(
            *spy.log.borrow(),
            vec![0x47, 0x30, 0x46, 0x49],
            "Apple's order, Input 0x30 after 0x47"
        );
        assert_eq!(d.outcome, SafeRead::Complete);
        for banned in ["0x4c", "0xfe", "0x01", "0x4C", "0xFE"] {
            assert!(!d.report.raw.contains_key(banned), "{banned}");
        }
    }

    #[test]
    fn text_and_json_are_decoded() {
        let d = collect(&fixture());
        let t = to_text(&d);
        assert!(t.contains("0x47  63        battery 99 %"), "{t}");
        assert!(t.contains("cell voltage 3002 mV (measured)"), "{t}");
        assert!(t.contains("filtered voltage 2953 mV (measured)"), "{t}");
        assert!(
            t.contains("input 0x30  01        battery state low (GET Input, Apple R2)"),
            "{t}"
        );
        let j = to_json(&d);
        assert_eq!(j["outcome"], "complete");
        assert_eq!(j["reports"]["0x46"]["hex"], "ba0b");
        assert_eq!(j["reports"]["input 0x30"]["hex"], "01");
        assert_eq!(j["reports"].as_object().unwrap().len(), 4);
    }

    #[test]
    fn stops_at_the_first_failure() {
        let f = akm_core::decode::Fixture::new()
            .with(&[0x47, 50])
            .with_input(&[0x30, 0]); // 0x46 answers NotFound
        let spy = Spy {
            inner: &f,
            log: RefCell::new(vec![]),
        };
        let d = collect(&spy);
        assert_eq!(d.outcome, SafeRead::Partial);
        assert_eq!(
            *spy.log.borrow(),
            vec![0x47, 0x30, 0x46],
            "0x49 is not tried after a failure"
        );
        assert!(to_text(&d).contains("0x49  (not read)"));
        assert!(d.report.incomplete);
    }
}
