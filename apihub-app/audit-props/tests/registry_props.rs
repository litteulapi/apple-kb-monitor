//! Propriétés du registre des rapports HID et du contrôle de version du
//! firmware (#227) : décodeurs totaux (jamais de panique), bornés, et classes
//! de sécurité respectées pour les 256 ids.

use akm_core::decode::{Fixture, HidSource};
use akm_core::firmware::{self, FirmwareStatus};
use akm_core::read_policy::{self, build_report_safe};
use akm_core::registry::{self, BatteryState, Safety, Thresholds};
use akm_core::report::{KbFirmware, KbWake};
use proptest::prelude::*;
use std::cell::RefCell;
use std::io;
use std::time::Instant;

const UEVENT: &str = "DRIVER=apple\nHID_ID=0005:000005AC:00000256\nHID_NAME=Clavier\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n";

struct Spy<'a>(&'a Fixture, RefCell<Vec<u8>>);
impl HidSource for Spy<'_> {
    fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
        self.1.borrow_mut().push(id);
        self.0.feature(id)
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Chaque décodeur de chaque entrée est total sur des octets arbitraires.
    #[test]
    fn every_decoder_is_total(payload in proptest::collection::vec(any::<u8>(), 0..80)) {
        for e in registry::all() {
            if let Some(d) = e.decode_payload(&payload) {
                let _ = e.render(&d);
            }
        }
    }

    /// Des seuils acceptés sont strictement décroissants et non nuls, les marges sont cohérentes.
    #[test]
    fn accepted_thresholds_are_strictly_decreasing(p in proptest::collection::vec(any::<u8>(), 0..16), mv in 0u32..10_000) {
        if let Some(t) = Thresholds::parse(&p) {
            let a = t.as_array();
            prop_assert!(a[3] > 0 && a[0] > a[1] && a[1] > a[2] && a[2] > a[3]);
            let m = t.margins(mv);
            prop_assert_eq!(m[0] - m[1], i32::from(a[1]) - i32::from(a[0]));
            prop_assert_eq!(m[2] - m[3], i32::from(a[3]) - i32::from(a[2]));
            // niveau monotone avec la tension
            let lvl = |v: u32| t.level(v) as u8;
            prop_assert!(lvl(mv.saturating_add(1)) <= lvl(mv), "level must not worsen when the voltage rises");
        }
    }

    /// Courbe d'affichage Apple : bornée, monotone sur 0..=100, constante au-delà.
    #[test]
    fn apple_curve_bounded_and_monotone(a in any::<u8>(), b in any::<u8>()) {
        let (pa, pb) = (registry::apple_display_percent(a), registry::apple_display_percent(b));
        prop_assert!((0.0..=100.0).contains(&pa));
        if a <= b && b <= 100 {
            prop_assert!(pa <= pb + 1e-9);
        }
        if a >= 100 {
            prop_assert_eq!(pa, 100.0);
        }
    }

    #[test]
    fn battery_state_total(b in any::<u8>()) {
        let s = BatteryState::from_byte(b);
        prop_assert_eq!(matches!(s, BatteryState::Invalid(_)), b > 3);
        prop_assert!(!s.as_str().is_empty());
    }

    /// Le contrôle de version est cohérent pour n'importe quel PID et n'importe quelle version.
    #[test]
    fn firmware_assessment_is_consistent(pid in proptest::option::of(any::<u32>()), v in proptest::option::of(any::<u16>())) {
        let a = firmware::assess(pid, v);
        match a.status {
            FirmwareStatus::UpToDate => prop_assert_eq!(v, a.latest),
            FirmwareStatus::UpdateAvailable { latest } => {
                prop_assert_eq!(Some(latest), a.latest);
                prop_assert!(v.is_some_and(|x| x < latest));
            }
            FirmwareStatus::Unknown => {}
        }
        // Un modèle inconnu n'a jamais de « dernière version ».
        if a.latest.is_none() {
            prop_assert_eq!(a.status, FirmwareStatus::Unknown);
        }
        let mut fw = KbFirmware { version: v.map(firmware::hex), ..Default::default() };
        firmware::assess_report(pid, &mut fw);
        prop_assert_eq!(fw.status.as_str(), a.status.as_str());
        for s in [firmware::summary_fr(&fw), firmware::summary_en(&fw)].into_iter().flatten() {
            let l = s.to_lowercase();
            prop_assert!(!l.contains("flash"));
        }
    }

    /// Texte hexadécimal arbitraire : le parseur de version ne panique pas, et l'aller-retour est exact.
    #[test]
    fn version_parse_never_panics(s in ".{0,12}", v in any::<u16>()) {
        let _ = firmware::parse_hex(&s);
        prop_assert_eq!(firmware::parse_hex(&firmware::hex(v)), Some(v));
    }
}

proptest! {
    // Chaque cas dort ~4 s (espacement de 1 s entre 5 requêtes) : peu de cas.
    #![proptest_config(ProptestConfig::with_cases(3))]

    /// Quelles que soient les réponses, une rafale du démon ne demande que des ids
    /// SafeRead / OncePerConnection, ne lit pas deux fois un id « une fois par connexion »
    /// et n'émet jamais un id NeverRead / NeverWrite.
    #[test]
    fn daemon_burst_never_reaches_a_forbidden_class(
        v4f in proptest::collection::vec(any::<u8>(), 0..6),
        v60 in proptest::collection::vec(any::<u8>(), 0..12),
    ) {
        let mut f = Fixture::new().with(&[0x47, 99]).with(&[0x46, 0xBA, 0x0B]).with(&[0x49, 0x89, 0x0B]);
        let mut a = vec![0x4F]; a.extend(&v4f); f = f.with(&a);
        let mut b = vec![0x60]; b.extend(&v60); f = f.with(&b);
        for id in 0..=255u8 {
            if ![0x47, 0x46, 0x49, 0x4F, 0x60].contains(&id) {
                f = f.with(&[id, 1, 2, 3, 4, 5, 6, 7, 8]);
            }
        }
        let spy = Spy(&f, RefCell::new(Vec::new()));
        let dir = std::env::temp_dir().join(format!("akm-props-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        read_policy::note_connection();
        let _ = build_report_safe(UEVENT, None, &spy, KbWake::default(), Instant::now());
        read_policy::note_input();
        let (rep, _) = build_report_safe(UEVENT, None, &spy, KbWake::default(), Instant::now());
        let _ = std::fs::remove_dir_all(&dir);
        let sent = spy.1.borrow().clone();
        prop_assert!(sent.len() >= 3, "the burst must really read: {sent:?}");
        for id in &sent {
            prop_assert!(matches!(registry::classify_feature(*id), Safety::SafeRead | Safety::OncePerConnection), "{id:#04x}");
        }
        prop_assert_eq!(sent.iter().filter(|&&i| i == 0x4F).count() <= 1, true);
        prop_assert_eq!(sent.iter().filter(|&&i| i == 0x60).count() <= 1, true);
        // valeurs publiées bornées
        if let Some(t) = rep.battery.thresholds {
            let a = t.as_array();
            prop_assert!(a[0] > a[1] && a[1] > a[2] && a[2] > a[3] && a[3] > 0);
        }
        prop_assert!(["up_to_date", "update_available", "unknown"].contains(&rep.firmware.status.as_str()));
    }
}

/// Pour les 256 ids, la porte de lecture suit exactement la classe.
#[test]
fn read_gate_equals_class_for_all_ids() {
    for id in 0..=255u8 {
        let c = registry::classify_feature(id);
        assert_eq!(registry::check_read(id).is_ok(), matches!(c, Safety::SafeRead | Safety::OncePerConnection));
        assert_eq!(registry::check_write(id, registry::Direction::Feature).is_ok(), id == 0x40, "seule l'ecriture Apple 0x40 est permise");
    }
}
