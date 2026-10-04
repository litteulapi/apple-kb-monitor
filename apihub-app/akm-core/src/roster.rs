//! The keyboards known to the daemon, side by side.

use serde::{Deserialize, Serialize};

/// Keyboards listed (a menu and a JSON document stay bounded).
pub const MAX_DEVICES: usize = 6;

/// One keyboard of the roster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceSummary {
    /// Address, upper case.
    pub mac: String,
    /// Name to show (alias, else the keyboard's own name, else the address).
    pub name: String,
    pub connected: bool,
    /// Percentage 0..100; None = unknown.
    pub battery: Option<f64>,
    /// The keyboard the daemon reads itself (full telemetry in `keyboard`).
    pub primary: bool,
}

/// Build the roster: `primary` first, then `others` by name.
#[must_use]
pub fn merge(primary: Option<DeviceSummary>, others: Vec<DeviceSummary>) -> Vec<DeviceSummary> {
    let mut out: Vec<DeviceSummary> = Vec::new();
    if let Some(mut p) = primary {
        p.mac = p.mac.to_ascii_uppercase();
        p.primary = true;
        out.push(p);
    }
    let mut rest: Vec<DeviceSummary> = others
        .into_iter()
        .map(|mut d| {
            d.mac = d.mac.to_ascii_uppercase();
            d.primary = false;
            d
        })
        .filter(|d| !out.iter().any(|p| p.mac == d.mac))
        .collect();
    rest.sort_by(|a, b| a.name.cmp(&b.name).then(a.mac.cmp(&b.mac)));
    rest.dedup_by(|a, b| a.mac == b.mac);
    out.extend(rest);
    out.truncate(MAX_DEVICES);
    out
}

/// The connected keyboard with the lowest known battery: the one the tray shows.
#[must_use]
pub fn weakest(devices: &[DeviceSummary]) -> Option<&DeviceSummary> {
    devices
        .iter()
        .filter(|d| d.connected)
        .filter_map(|d| d.battery.filter(|b| b.is_finite()).map(|b| (d, b)))
        .fold(
            None,
            |best: Option<(&DeviceSummary, f64)>, (d, b)| match best {
                Some((_, lowest)) if lowest <= b => best,
                _ => Some((d, b)),
            },
        )
        .map(|(d, _)| d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb(mac: &str, name: &str, connected: bool, battery: Option<f64>) -> DeviceSummary {
        DeviceSummary {
            mac: mac.into(),
            name: name.into(),
            connected,
            battery,
            primary: false,
        }
    }

    const A: &str = "AA:BB:CC:DD:EE:F1";
    const B: &str = "AA:BB:CC:DD:EE:F2";

    #[test]
    fn the_primary_comes_first_and_is_not_listed_twice() {
        let r = merge(
            Some(kb(A, "Desk", true, Some(80.0))),
            vec![
                kb("aa:bb:cc:dd:ee:f2", "Salon", true, Some(40.0)),
                kb("aa:bb:cc:dd:ee:f1", "Desk (BlueZ)", true, Some(79.0)),
            ],
        );
        assert_eq!(r.len(), 2);
        assert_eq!(
            (r[0].mac.as_str(), r[0].primary, r[0].battery),
            (A, true, Some(80.0))
        );
        assert_eq!((r[1].mac.as_str(), r[1].primary), (B, false));
        let r = merge(
            None,
            vec![kb(B, "Salon", false, None), kb(A, "Desk", true, None)],
        );
        assert_eq!(
            r.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["Desk", "Salon"]
        );
        assert!(r.iter().all(|d| !d.primary));
        assert!(
            merge(None, Vec::new()).is_empty(),
            "{:?}",
            merge(None, Vec::new())
        );
    }

    #[test]
    fn the_weakest_connected_keyboard_is_the_one_shown() {
        let r = merge(
            Some(kb(A, "Desk", true, Some(80.0))),
            vec![kb(B, "Salon", true, Some(12.0))],
        );
        assert_eq!(weakest(&r).unwrap().mac, B);
        let r = merge(
            Some(kb(A, "Desk", true, Some(80.0))),
            vec![kb(B, "Salon", false, Some(12.0))],
        );
        assert_eq!(weakest(&r).unwrap().mac, A);
        assert_eq!(r[0].battery, Some(80.0));
        // Unknown batteries are never "the weakest"; a tie goes to the primary.
        let r = merge(
            Some(kb(A, "Desk", true, Some(50.0))),
            vec![kb(B, "Salon", true, None)],
        );
        assert_eq!(weakest(&r).unwrap().mac, A);
        let r = merge(
            Some(kb(A, "Desk", true, Some(50.0))),
            vec![kb(B, "Salon", true, Some(50.0))],
        );
        assert_eq!(weakest(&r).unwrap().mac, A);
        assert_eq!(
            weakest(&merge(None, vec![kb(B, "Salon", false, Some(5.0))])),
            None
        );
        assert_eq!(weakest(&[]), None);
    }

    #[test]
    fn the_roster_is_bounded() {
        let others = (0..20)
            .map(|i| {
                kb(
                    &format!("AA:BB:CC:DD:EE:{i:02X}"),
                    &format!("kb{i:02}"),
                    true,
                    None,
                )
            })
            .collect();
        assert_eq!(merge(None, others).len(), MAX_DEVICES);
    }
}
