//! Propriétés des analyseurs : config.toml, historique JSONL, descripteur HID,
//! rapports d'entrée passifs, alias, calibration.

use akm_core::alias::validate;
use akm_core::calibration::{calibration_valid, estimate_percentage_mv, parse_calibration};
use akm_core::config;
use akm_core::discover::{decode_wake, parse_rdesc};
use akm_core::history;
use proptest::prelude::*;

/// Descripteur clavier minimal : Usage Page(Generic Desktop) Usage(Keyboard)
/// Collection(Application) Report ID 1, Input, End Collection.
const KBD_RDESC: [u8; 17] = [
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x85, 0x01, 0x95, 0x08, 0x75, 0x01, 0x81, 0x02, 0xC0,
    0x00, 0x00,
];

/// Texte mêlant caractères quelconques et invisibles/bidi connus.
fn alias_input() -> impl Strategy<Value = String> {
    let special = prop::sample::select(vec![
        '\u{200B}', '\u{200E}', '\u{202E}', '\u{2066}', '\u{FEFF}', '\u{061C}', '\u{3164}',
        '\u{2800}', '\u{00AD}', '\u{0085}', '\0', '\n', ' ', 'a', 'é', '\u{1F3B9}',
    ]);
    prop::collection::vec(prop_oneof![3 => special, 1 => any::<char>()], 0..90)
        .prop_map(|v| v.into_iter().collect())
}

fn configish() -> impl Strategy<Value = String> {
    let line = prop_oneof![
        Just("[alerts]".to_string()),
        Just("[battery]".to_string()),
        Just("[notifications]".to_string()),
        "[a-z_]{0,12} ?= ?[-0-9a-z_.\"\\[\\], #]{0,30}".prop_map(|s| s),
        "thresholds = \\[[-0-9, _]{0,30}\\]".prop_map(|s| s),
        "hysteresis = [-0-9.e_+a-z]{0,12}".prop_map(|s| s),
        "critical = [-0-9_]{0,22}".prop_map(|s| s),
        any::<String>().prop_map(|s| s.chars().take(40).collect::<String>()),
    ];
    prop::collection::vec(line, 0..20).prop_map(|v| v.join("\n"))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4000, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn config_parse_never_panics_and_normalises(s in configish()) {
        let (cfg, _warn) = config::parse(&s);
        let t = cfg.alerts.thresholds();
        prop_assert!(t.windows(2).all(|w| w[0] > w[1]), "seuils non triés/uniques {t:?}");
        prop_assert!(t.iter().all(|x| (1..=99).contains(x)));
        prop_assert!(cfg.alerts.hysteresis.is_finite());
        prop_assert!((1.0..=20.0).contains(&cfg.alerts.hysteresis));
        prop_assert!(cfg.alerts.critical_at <= 99);
    }

    #[test]
    fn config_parse_arbitrary_text(s in any::<String>()) {
        let _ = config::parse(&s);
    }

    #[test]
    fn history_parse_arbitrary_text_never_panics(s in any::<String>()) {
        let _ = history::parse(&s);
    }

    /// Lignes JSON bien formées mais aux valeurs extrêmes.
    #[test]
    fn history_parse_extreme_numbers(
        ts in any::<u64>(), pct in -1e308f64..1e308, v in -1e308f64..1e308,
        mv in any::<u32>(), schema in any::<u8>(),
    ) {
        let line = format!(
            "{{\"ts\":{ts},\"pct\":{pct:e},\"voltage\":{v:e},\"mv_0x46\":{mv},\"schema\":{schema}}}"
        );
        let e = history::parse(&line);
        prop_assert!(e.len() <= 1);
    }

    #[test]
    fn rdesc_parser_never_panics(d in prop::collection::vec(any::<u8>(), 0..400)) {
        let _ = parse_rdesc(&d);
    }

    #[test]
    fn wake_and_calibration_parsers_never_panic(b in prop::collection::vec(any::<u8>(), 0..40)) {
        let _ = decode_wake(&b);
        if let Some(c) = parse_calibration(&b) {
            prop_assert!(calibration_valid(&c));
        }
    }

    /// Alias : si accepté, aucun caractère de contrôle, borné, idempotent.
    #[test]
    fn alias_accepted_is_clean_and_idempotent(s in alias_input()) {
        if let Ok(out) = validate(&s) {
            prop_assert!(out.chars().all(|c| !c.is_control()));
            prop_assert!(out.chars().count() <= 64 && out.len() <= 248);
            prop_assert_eq!(validate(&out).unwrap(), out);
        }
    }

    /// Estimation de calibration : jamais NaN/inf, bornée, monotone.
    #[test]
    fn calibration_estimate_bounded_and_monotone(
        t in prop::array::uniform4(any::<u16>()), a in any::<f64>(), b in any::<f64>(),
    ) {
        for mv in [a, b] {
            if let Some(p) = estimate_percentage_mv(mv, &t) {
                prop_assert!(p.is_finite() && (0.0..=100.0).contains(&p), "p={p}");
            }
        }
        if calibration_valid(&t) && a.is_finite() && b.is_finite() {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let (pl, ph) = (estimate_percentage_mv(lo, &t).unwrap(), estimate_percentage_mv(hi, &t).unwrap());
            prop_assert!(pl <= ph + 1e-9, "non monotone: f({lo})={pl} > f({hi})={ph}");
        }
    }
}

#[test]
fn rdesc_keyboard_prefixes_never_panic_and_full_is_keyboard() {
    let full = parse_rdesc(&KBD_RDESC);
    assert!(full.keyboard && full.has_input_report(1));
    for n in 0..=KBD_RDESC.len() {
        let _ = parse_rdesc(&KBD_RDESC[..n]);
    }
}

/// Défaut de couverture de la validation d'alias : U+061C (marque de lettre
/// arabe, contrôle bidi) et les remplisseurs invisibles (U+3164, U+2800) passent.
#[test]
fn alias_rejects_remaining_bidi_and_blank_fillers() {
    for bad in ["a\u{061C}b", "\u{3164}", "x\u{2800}", "a\u{00AD}b", "a\u{E0041}b", "a\u{FE0F}\u{180E}"] {
        assert!(validate(bad).is_err(), "{bad:?} accepté");
    }
}
