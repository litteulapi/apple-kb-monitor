//! Propriétés du décodeur des rapports Feature (BCM2042) et de la lecture sûre.
//! Entrées aléatoires ou tronquées : jamais de panique, valeurs publiées bornées,
//! la liste blanche de la lecture sûre (#177) n'est jamais dépassée.

use akm_core::calibration::PLAUSIBLE_MV;
use akm_core::decode::{build_report, decode_bcm2042_with, DecodeOptions, Fixture, HidSource};
use akm_core::read_policy::{self, build_report_safe};
use akm_core::report::{KbReport, KbWake};
use proptest::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::time::Instant;

const UEVENT: &str = "DRIVER=apple\nHID_ID=0005:000005AC:00000256\nHID_NAME=Clavier\nHID_UNIQ=04:db:56:ca:42:ee\n";
const REAL_FRAMES: &str = include_str!("../../../tests/live/re/a1314_iso_frames.hex");

/// Ids lus par le décodeur + quelques ids hors table.
const IDS: [u8; 18] = [
    0xEA, 0x47, 0x46, 0x49, 0xF5, 0x5A, 0x4F, 0xFF, 0x51, 0x52, 0x53, 0x4C, 0xF4, 0x09, 0x60,
    0xEB, 0x30, 0x00,
];

/// Source pilotée par un tableau : None = erreur, Some(octets) = réponse brute
/// (l'octet 0 n'est PAS forcément l'id : le noyau peut répondre n'importe quoi).
#[derive(Debug)]
struct Table(Vec<Option<Vec<u8>>>, RefCell<Vec<u8>>);

impl HidSource for Table {
    fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
        self.1.borrow_mut().push(id);
        let i = IDS.iter().position(|&x| x == id);
        match i.and_then(|i| self.0[i].clone()) {
            Some(b) => Ok(b),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "absent")),
        }
    }
}

fn frame() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![
        1 => Just(None),
        6 => prop::collection::vec(any::<u8>(), 0..24).prop_map(Some),
        1 => prop::collection::vec(any::<u8>(), 0..300).prop_map(Some),
    ]
}

fn table() -> impl Strategy<Value = Table> {
    prop::collection::vec(frame(), IDS.len()).prop_map(|v| Table(v, RefCell::new(Vec::new())))
}

fn check_report(r: &KbReport) {
    if let Some(p) = r.battery.percentage {
        assert!((0.0..=100.0).contains(&p), "pct hors bornes: {p}");
    }
    if let Some(mv) = r.battery.voltage_mv {
        assert!(PLAUSIBLE_MV.contains(&u16::try_from(mv).unwrap()), "mV implausible {mv}");
    }
    if let Some(v) = r.battery.voltage {
        assert!(v.is_finite() && v >= 0.0, "tension {v}");
    }
    for e in [r.battery.percentage_estimate, r.battery.percentage_interpolated]
        .into_iter()
        .flatten()
    {
        assert!(e.is_finite() && (0.0..=100.0).contains(&e), "estimation {e}");
    }
    if let Some(n) = &r.device.name {
        assert!(!n.contains('\0'), "NUL dans le nom");
    }
    if let Some(a) = &r.bluetooth.paired_host_addr {
        assert_eq!(a.len(), 17, "adresse {a}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4000, failure_persistence: None, ..ProptestConfig::default() })]

    /// Aucune panique, rapport borné, 0x4C jamais publié sans l'option (#123).
    #[test]
    fn decoder_never_panics_and_bounds_hold(t in table()) {
        let mut r = KbReport::default();
        let _ = decode_bcm2042_with(&t, &mut r, &DecodeOptions::default());
        check_report(&r);
        prop_assert!(!r.raw.contains_key("0x4c"), "0x4C publié sans reveal_pairing_bytes");
        let full = build_report(UEVENT, None, &t, KbWake::default());
        if let Some(rep) = full { check_report(&rep); }
    }

    /// La lecture sûre (#177) n'émet que 0x47 / 0x46 / 0x49.
    #[test]
    fn safe_read_only_requests_the_allow_list(t in table()) {
        let (rep, _) = build_report_safe(UEVENT, None, &t, KbWake::default(), Instant::now());
        for id in t.1.borrow().iter() {
            prop_assert!(read_policy::is_allowed(*id), "id {id:#04x} hors liste blanche");
        }
        prop_assert!(!rep.raw.contains_key("0x4c"));
        if let Some(p) = rep.battery.percentage {
            prop_assert!((0.0..=100.0).contains(&p));
        }
    }

    /// Le chargeur de dumps hexa ne doit pas paniquer sur une entrée quelconque.
    #[test]
    fn hex_dump_parser_never_panics(s in "[0-9a-fA-F \\n#é€]{0,40}") {
        let _ = Fixture::from_hex_dump(&s);
    }
}

/// Trames réelles (A1314 ISO) tronquées à chaque longueur possible : jamais de panique.
#[test]
fn real_frames_truncated_at_every_length() {
    let real = Fixture::from_hex_dump(REAL_FRAMES).unwrap();
    let mut frames: HashMap<u8, Vec<u8>> = HashMap::new();
    for id in 0..=255u8 {
        if let Ok(b) = real.feature(id) {
            frames.insert(id, b);
        }
    }
    assert!(frames.len() >= 8, "fixture réelle attendue");
    for cut in 0..=40usize {
        let mut tbl: Vec<Option<Vec<u8>>> = Vec::new();
        for id in IDS {
            tbl.push(frames.get(&id).map(|b| b[..cut.min(b.len())].to_vec()));
        }
        let t = Table(tbl, RefCell::new(Vec::new()));
        let mut r = KbReport::default();
        let _ = decode_bcm2042_with(&t, &mut r, &DecodeOptions::default());
        check_report(&r);
        let _ = build_report_safe(UEVENT, None, &t, KbWake::default(), Instant::now());
    }
}

/// Reproducteur déterministe : un caractère multi-octets fait paniquer
/// `Fixture::from_hex_dump` (tranchage `&hex[i..i + 2]` hors frontière de char).
#[test]
fn hex_dump_multibyte_char_does_not_panic() {
    let r = std::panic::catch_unwind(|| Fixture::from_hex_dump("aéa"));
    assert!(r.is_ok(), "from_hex_dump a paniqué");
    assert!(r.unwrap().is_err());
}
