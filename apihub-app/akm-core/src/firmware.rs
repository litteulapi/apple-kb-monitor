//! Firmware version check.

use crate::report::KbFirmware;

/// Date of the last review of [`KNOWN_FIRMWARE`] (ISO 8601).
pub const TABLE_DATE: &str = "2026-10-01";

const OLDER_WITH_UPDATE: &[u16] = &[0x0044, 0x0046];

/// Latest public firmware of a group of product ids.
#[derive(Debug, Clone, Copy)]
pub struct KnownFirmware {
    /// Bluetooth product ids (`HID_ID` product field).
    pub pids: &'static [u16],
    /// Latest public version known (what `0x4F` reports).
    pub latest: u16,
    /// Where the entry comes from.
    pub source: &'static str,
    /// `latest` is an Apple public update; false = only measured here, never "up to date" (R4).
    pub public: bool,
}

/// The embedded table. Keep `docs/FIRMWARE.md` in step with it.
pub const KNOWN_FIRMWARE: &[KnownFirmware] = &[
    KnownFirmware {
        pids: &[0x0255, 0x0256, 0x0257],
        latest: 0x0050,
        source: "measured on an A1314 ISO (05AC:0256, 0x4F = 0x0050, 2026-10-01); \
no public Apple updater is known for these product ids",
        public: false,
    },
    KnownFirmware {
        pids: &[0x0239, 0x023A, 0x023B],
        latest: 0x0050,
        source: "Apple \"2009 Aluminum Keyboard Firmware Update\" \
(support.apple.com/en-us/106755, FWVersion=80=0x50; replaces 0x44 / 0x46)",
        public: true,
    },
];

/// Result of the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareStatus {
    UpToDate,
    UpdateAvailable {
        latest: u16,
    },
    /// Model not in the table, version not read, or version out of the table's knowledge.
    Unknown,
}

impl FirmwareStatus {
    /// Stable identifier (JSON `firmware.status`, D-Bus `FirmwareStatus`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up_to_date",
            Self::UpdateAvailable { .. } => "update_available",
            Self::Unknown => "unknown",
        }
    }
}

/// Outcome with the data that justifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    pub status: FirmwareStatus,
    pub version: Option<u16>,
    /// Latest public version known for this product id.
    pub latest: Option<u16>,
    pub source: Option<&'static str>,
}

/// Table entry of a product id.
#[must_use]
pub fn known_for(pid: u16) -> Option<&'static KnownFirmware> {
    KNOWN_FIRMWARE.iter().find(|k| k.pids.contains(&pid))
}

/// Compare `version` (report `0x4F`) with the table for product id `pid`.
pub fn assess(pid: Option<u32>, version: Option<u16>) -> Assessment {
    let known = pid.and_then(|p| u16::try_from(p).ok()).and_then(known_for);
    let public = known.filter(|k| k.public);
    let status = match (public, version) {
        (Some(k), Some(v)) if v == k.latest => FirmwareStatus::UpToDate,
        (Some(k), Some(v)) if OLDER_WITH_UPDATE.contains(&v) => {
            FirmwareStatus::UpdateAvailable { latest: k.latest }
        }
        _ => FirmwareStatus::Unknown,
    };
    Assessment {
        status,
        version,
        latest: public.map(|k| k.latest),
        source: known.map(|k| k.source),
    }
}

/// `0x0050` -> `"0x0050"`.
#[must_use]
pub fn hex(v: u16) -> String {
    format!("0x{v:04X}")
}

/// Parse `"0x0050"` / `"0050"` back.
#[must_use]
pub fn parse_hex(s: &str) -> Option<u16> {
    let t = s.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    (!t.is_empty() && t.len() <= 4)
        .then(|| u16::from_str_radix(t, 16).ok())
        .flatten()
}

impl Assessment {
    /// Fill the firmware block of a report.
    pub fn apply(&self, fw: &mut KbFirmware) {
        fw.version = self.version.map(hex);
        fw.version_hex = fw.version.clone();
        fw.latest_known = self.latest.map(hex);
        fw.status = self.status.as_str().to_string();
        fw.source = self.source.map(str::to_string);
        fw.table_date = Some(TABLE_DATE.to_string());
    }
}

/// Assess a report's firmware block in place from the product id.
pub fn assess_report(pid: Option<u32>, fw: &mut KbFirmware) {
    let version = fw.version.as_deref().and_then(parse_hex);
    assess(pid, version).apply(fw);
}

/// One line for humans: `Firmware: 0x0050 - up to date`.
#[must_use]
pub fn summary(fw: &KbFirmware) -> Option<String> {
    use crate::tr;
    let v = fw.version.as_deref()?;
    Some(match (fw.status.as_str(), fw.latest_known.as_deref()) {
        ("up_to_date", _) => tr!(
            "Firmware: {v} - up to date (latest public version known)",
            v = v
        ),
        ("update_available", Some(l)) => tr!(
            "Firmware: {v} - update available from Apple (latest known: {l})",
            v = v,
            l = l
        ),
        _ if fw.source.is_some() => {
            tr!(
                "Firmware: {v} - no public Apple update for this model",
                v = v
            )
        }
        _ => tr!(
            "Firmware: {v} - unknown (model or version not in the table)",
            v = v
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_firmware_of_every_known_pid_is_up_to_date() {
        for pid in [0x0239, 0x023A, 0x023B] {
            let a = assess(Some(pid), Some(0x0050));
            assert_eq!(a.status, FirmwareStatus::UpToDate, "{pid:#06x}");
            assert_eq!(a.latest, Some(0x0050));
            assert!(a.source.is_some());
        }
    }

    #[test]
    fn documented_older_versions_have_an_update() {
        for v in [0x0044, 0x0046] {
            assert_eq!(
                assess(Some(0x0256), Some(v)).status,
                FirmwareStatus::Unknown
            );
            assert_eq!(
                assess(Some(0x0239), Some(v)).status,
                FirmwareStatus::UpdateAvailable { latest: 0x0050 }
            );
        }
    }

    #[test]
    fn what_the_table_cannot_tell_is_unknown() {
        // R4: 0x0255-0x0257 were measured here, no Apple reference: never "up to date".
        for pid in [0x0255, 0x0256, 0x0257] {
            let a = assess(Some(pid), Some(0x0050));
            assert_eq!(
                (a.status, a.latest),
                (FirmwareStatus::Unknown, None),
                "{pid:#06x}"
            );
        }
        assert_eq!(
            assess(Some(0x0256), Some(0x0051)).status,
            FirmwareStatus::Unknown
        );
        assert_eq!(
            assess(Some(0x0256), Some(0x0010)).status,
            FirmwareStatus::Unknown
        );
        assert_eq!(
            assess(Some(0x022C), Some(0x0050)).status,
            FirmwareStatus::Unknown
        );
        assert_eq!(
            assess(Some(0x029C), Some(0x0050)).status,
            FirmwareStatus::Unknown
        );
        assert_eq!(assess(None, Some(0x0050)).status, FirmwareStatus::Unknown);
        assert_eq!(assess(Some(0x0256), None).status, FirmwareStatus::Unknown);
        assert_eq!(
            assess(Some(0x1_0256), Some(0x0050)).status,
            FirmwareStatus::Unknown
        );
        assert_eq!(assess(Some(0x022C), Some(0x0050)).latest, None);
    }

    #[test]
    fn pids_are_unique_across_the_table() {
        let mut all: Vec<u16> = KNOWN_FIRMWARE
            .iter()
            .flat_map(|k| k.pids.iter().copied())
            .collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n);
    }

    #[test]
    fn hex_roundtrip_and_garbage() {
        assert_eq!(hex(0x50), "0x0050");
        assert_eq!(parse_hex("0x0050"), Some(0x50));
        assert_eq!(parse_hex("0050"), Some(0x50));
        assert_eq!(parse_hex(""), None);
        assert_eq!(parse_hex("0x"), None);
        assert_eq!(parse_hex("0x12345"), None);
        assert_eq!(parse_hex("zz"), None);
        for v in [0u16, 1, 0x50, 0xFFFF] {
            assert_eq!(parse_hex(&hex(v)), Some(v));
        }
    }

    #[test]
    fn report_block_is_filled_and_summarised() {
        let mut fw = KbFirmware {
            version: Some("0x0050".into()),
            ..Default::default()
        };
        assess_report(Some(0x0239), &mut fw);
        assert_eq!(fw.status, "up_to_date");
        assert_eq!(fw.version_hex.as_deref(), Some("0x0050"));
        assert_eq!(fw.latest_known.as_deref(), Some("0x0050"));
        assert_eq!(fw.table_date.as_deref(), Some(TABLE_DATE));
        assert_eq!(
            summary(&fw).unwrap(),
            "Firmware: 0x0050 - up to date (latest public version known)"
        );
        assert!(summary(&fw).unwrap().contains("up to date"));
        let mut old = KbFirmware {
            version: Some("0x0044".into()),
            ..Default::default()
        };
        assess_report(Some(0x0239), &mut old);
        assert_eq!(old.status, "update_available");
        assert!(summary(&old)
            .unwrap()
            .contains("update available from Apple"));
        let mut none = KbFirmware::default();
        assess_report(Some(0x0256), &mut none);
        assert_eq!(none.status, "unknown");
        assert_eq!(summary(&none), None);
    }

    #[test]
    fn never_proposes_to_flash() {
        // The vocabulary of the summaries never invites a flash.
        let mut fw = KbFirmware {
            version: Some("0x0046".into()),
            ..Default::default()
        };
        assess_report(Some(0x0256), &mut fw);
        for s in [summary(&fw).unwrap(), summary(&fw).unwrap()] {
            let l = s.to_lowercase();
            assert!(!l.contains("flash") && !l.contains("install"));
        }
    }
}
