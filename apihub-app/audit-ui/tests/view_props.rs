//! Propriétés des fonctions de présentation (docs/AUDIT-UI.md §6).
//!
//! * `src/view.rs` (fenêtre egui) et `apple-kb-monitord/src/tray/view.rs`
//!   (tray du démon) sont inclus tels quels par `#[path]` : c'est le code livré
//!   qui est testé, pas une copie.
//! * Le calcul de position du graphe et la légende vivent dans
//!   `src/main.rs::draw_battery_history` (méthode privée, non incluable) : les
//!   deux formules sont recopiées à l'identique dans [`chart`] avec la ligne
//!   d'origine en référence ; si `main.rs` change, ce module doit suivre.
//!
//! Invariants : jamais de panique, jamais de valeur non finie à l'écran,
//! textes bornés en longueur, légende bornée en largeur, nombre d'étiquettes
//! d'axe constant (5) quelles que soient les données.

#![allow(dead_code)]

// `src/view.rs` imports `crate::i18n` (gettext catalogue embedded at build).
#[path = "../../src/i18n.rs"]
mod i18n;

#[path = "../../src/view.rs"]
mod view;

#[path = "../../apple-kb-monitord/src/tray/view.rs"]
mod tray_view;

use akm_core::chemistry::{estimate_charge, Chemistry};
use akm_core::report::{KbBattery, KbBluetooth, KbWake};
use akm_core::{KbReport, Snapshot};
use proptest::prelude::*;

/// Copie conforme des formules de `src/main.rs` (commit 8f73964).
mod chart {
    /// main.rs:483-485 — position x/y d'un point dans un tracé de largeur `w`
    /// et hauteur `h` dont l'origine est (0, 0).
    pub fn xy(t: f64, frac: f64, t_min: f64, t_max: f64, w: f32, h: f32) -> (f32, f32) {
        let t_range = (t_max - t_min).max(1.0);
        let x = ((t - t_min) / t_range) as f32 * w;
        let y = h - (frac.clamp(0.0, 1.0) as f32) * h;
        (x, y)
    }
    /// main.rs:509-512 — échelle propre de la tension.
    pub fn volt_scale(v: &[(f64, f64)]) -> (f64, f64, f64, f64) {
        let v_min = v.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let v_max = v.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let v_range = (v_max - v_min).max(0.1);
        (v_min, v_max, v_min - v_range * 0.05, v_max + v_range * 0.05)
    }
    /// main.rs:507,519 — textes de la légende.
    pub fn legend(v_min: f64, v_max: f64, with_voltage: bool) -> Vec<String> {
        let mut l = vec!["Battery %".to_string()];
        if with_voltage {
            l.push(format!("Voltage {v_min:.2}\u{2013}{v_max:.2} V (own scale)"));
        }
        l
    }
    /// main.rs:487 — graduations de l'axe des pourcentages.
    pub const Y_TICKS: [f64; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];
}

/// Flottants « hostiles » : NaN, ±∞, ±0, sous-normaux, extrêmes, valeurs usuelles.
fn hostile_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        Just(f64::NAN),
        Just(f64::INFINITY),
        Just(f64::NEG_INFINITY),
        Just(0.0),
        Just(-0.0),
        Just(f64::MIN_POSITIVE),
        Just(f64::MAX),
        Just(f64::MIN),
        Just(1e-300),
        -1e12..1e12f64,
        0.0..100.0f64,
        2.0..3.3f64,
        any::<f64>(),
    ]
}

fn ts() -> impl Strategy<Value = f64> {
    prop_oneof![
        Just(1_790_000_000.0),
        1_789_900_000.0..1_790_100_000.0f64,
        hostile_f64(),
    ]
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    // ── Graphe : points, positions, légende ─────────────────────────────────

    #[test]
    fn chart_points_are_finite_sorted_and_in_range(
        pts in prop::collection::vec((ts(), hostile_f64()), 0..400),
        cutoff in ts(),
        pct in any::<bool>(),
    ) {
        let out = view::chart_points(&pts, cutoff, 86_400.0, pct);
        prop_assert!(out.len() <= pts.len());
        for w in out.windows(2) {
            prop_assert!(w[0].0 <= w[1].0, "not sorted: {:?}", w);
        }
        for &(t, y) in &out {
            prop_assert!(t.is_finite() && y.is_finite());
            prop_assert!(t >= cutoff);
            if pct {
                prop_assert!((0.0..=100.0).contains(&y));
            } else {
                prop_assert!(y > 0.0 && y < 10.0);
            }
        }
    }

    /// Tout point retenu tombe DANS le tracé (aucune coordonnée NaN/∞ ou hors
    /// cadre), quelle que soit la plage de temps (nulle, minuscule, énorme).
    /// Domaine réel : `HistoryEntry::ts` est un u64 (main.rs:434 le convertit
    /// en f64) et la coupure vaut « maintenant − 24 h ».
    #[test]
    fn chart_geometry_stays_inside_the_plot(
        raw in prop::collection::vec((prop_oneof![any::<u64>(), 1_789_900_000u64..1_790_100_000], hostile_f64()), 2..300),
        w in 1.0f32..8000.0, h in 1.0f32..8000.0,
    ) {
        let pts: Vec<(f64, f64)> = raw.iter().map(|&(t, y)| (t as f64, y)).collect();
        let b = view::chart_points(&pts, 1_790_000_000.0 - 86_400.0, 86_400.0, true);
        prop_assume!(b.len() >= 2);
        let (t_min, t_max) = (b[0].0, b[b.len() - 1].0);
        for &(t, p) in &b {
            let (x, y) = chart::xy(t, p / 100.0, t_min, t_max, w, h);
            prop_assert!(x.is_finite() && y.is_finite(), "t={t} p={p}");
            prop_assert!((-0.01..=w + 0.01).contains(&x), "x={x} w={w}");
            prop_assert!((-0.01..=h + 0.01).contains(&y), "y={y} h={h}");
        }
        let v = view::chart_points(&pts, t_min, 86_400.0, false);
        if v.len() >= 2 {
            let (_, _, lo, hi) = chart::volt_scale(&v);
            prop_assert!(hi > lo && (hi - lo).is_finite());
            for &(_, volt) in &v {
                let f = (volt - lo) / (hi - lo);
                prop_assert!(f.is_finite() && (0.0..=1.0).contains(&f));
            }
        }
    }

    /// La légende garde au plus 2 entrées et 40 caractères chacune, quelles
    /// que soient les données (le défaut de la capture : des centaines de
    /// chiffres superposés, version 3.1.0-5 installée).
    #[test]
    fn chart_legend_is_bounded(
        pts in prop::collection::vec((ts(), hostile_f64()), 0..300),
    ) {
        let v = view::chart_points(&pts, f64::MIN, 86_400.0, false);
        let (v_min, v_max, _, _) = chart::volt_scale(&v);
        let legend = chart::legend(v_min, v_max, v.len() >= 2);
        prop_assert!(legend.len() <= 2);
        for l in &legend {
            prop_assert!(chars(l) <= 40, "{l:?}");
        }
        prop_assert_eq!(chart::Y_TICKS.len(), 5);
    }

    // ── Textes de la fenêtre ────────────────────────────────────────────────

    #[test]
    fn window_texts_never_panic_and_stay_short(
        p in hostile_f64(), dec in 0usize..3, age in any::<Option<u64>>(),
        r in any::<Option<i32>>(), tx in any::<Option<i32>>(),
        wake in hostile_f64(), count in any::<u64>(), mv in any::<u32>(),
    ) {
        // Pourcentage : seul le domaine 0..=100 est affiché (pct_source filtre).
        let shown = view::pct_source(&KbBattery { percentage: Some(p), ..Default::default() }).value();
        prop_assert!(chars(&view::pct_text(shown, dec)) <= 8);
        let f = view::pct_fraction(Some(p));
        prop_assert!(f.is_finite() && (0.0..=1.0).contains(&f));
        prop_assert!(chars(&view::age_text(age)) <= 24);
        prop_assert!(chars(&view::rssi_text(r)) <= 24);
        prop_assert!(view::rssi_bar_count(r) <= 4);
        prop_assert!(chars(&view::tx_power_text(tx)) <= 16);
        let w = view::wake_text(&KbWake { last_age_s: Some(wake), count });
        if let Some(w) = w {
            prop_assert!(chars(&w) <= 64, "{w}");
        }
        for chem in [Chemistry::Alkaline, Chemistry::Nimh, Chemistry::Lithium, Chemistry::Unknown] {
            let b = KbBattery { charge_estimate: estimate_charge(mv, chem), ..Default::default() };
            if let Some(t) = view::estimate_text(&b) {
                prop_assert!(chars(&t) <= 48, "{t}");
            }
        }
        // Tension mesurée : rapports 0x46/0xFF en mV sur 16 bits (≤ 65,535 V).
        let v = f64::from(mv % 65_536) / 1000.0;
        if v > 0.0 {
            prop_assert!(chars(&view::volts_text(v)) <= 8);
            let b = KbBattery { voltage: Some(v), ..Default::default() };
            prop_assert!(view::chemistry_text(&b).map_or(0, |t| chars(&t)) <= 48);
        }
    }

    // ── Tray du démon ───────────────────────────────────────────────────────

    #[test]
    fn tray_bucket_is_a_valid_icon_step(p in hostile_f64(), prev in any::<Option<u8>>()) {
        let b = tray_view::bucket(p, prev);
        prop_assert!(b <= 100 || Some(b) == prev);
    }

    /// Infobulle et menu bornés : nom (alias ≤ 64 caractères, #141) + 64
    /// caractères au plus par ligne ; jamais de « NaN » ni « inf ».
    #[test]
    fn tray_view_is_bounded(
        p in hostile_f64(), mv in any::<u16>(), connected in any::<bool>(),
        rssi in any::<Option<i32>>(), name in "[a-zA-Z0-9 _é<>&]{0,64}",
        model in "[a-zA-Z0-9 (),]{0,80}", charging in any::<bool>(), fr in any::<bool>(),
        last in any::<u64>(), now in any::<u64>(),
    ) {
        let mut k = KbReport::default();
        k.battery.percentage = Some(p);
        k.battery.voltage = Some(f64::from(mv) / 1000.0);
        k.radio.rssi_rel_db = rssi;
        k.device.model = Some(model.clone());
        k.device.alias = Some(name.clone());
        k.bluetooth = KbBluetooth { connected, ..Default::default() };
        let snap = Snapshot { connected, keyboard: Some(k), last_update: last, ..Default::default() };
        let lang = if fr { tray_view::Lang::Fr } else { tray_view::Lang::En };
        let v = tray_view::View::build(&snap, charging, None, lang);
        let bound = chars(&name).max(chars(&model)) * 2 + 64;
        prop_assert!(chars(&v.tooltip_title) <= bound, "{}", v.tooltip_title);
        for l in &v.tooltip_lines {
            prop_assert!(chars(l) <= bound, "{l}");
            // Only the numbers are checked: the model and the alias are
            // shown verbatim and may themselves spell « inf » or « NaN ».
            let mut nums = l.clone();
            for user in [&model, &name] {
                if !user.is_empty() {
                    nums = nums.replace(user.as_str(), "");
                }
            }
            prop_assert!(!nums.contains("NaN") && !nums.contains("inf"), "{l}");
        }
        for e in &v.menu {
            if let Some(tray_view::Prop::Str(s)) = e.get("label") {
                prop_assert!(chars(s) <= 2 * bound, "{s}");
            }
        }
        let body = v.tooltip_body(lang, now, true);
        prop_assert!(body.lines().count() <= 10);
        let clip = tray_view::clipboard_text(&snap, charging, lang, now);
        prop_assert!(clip.lines().count() <= 14);
    }
}

// ── Largeur réelle de la légende (polices egui, sans fenêtre) ───────────────

/// Mesure, avec les polices d'egui, la largeur de la légende la plus large
/// possible (main.rs:523-532 : texte 11 px + pastille 12 px + 14 px d'écart).
/// Elle doit tenir dans le cadre le plus étroit autorisé (fenêtre 500 px de
/// large au minimum → tracé ≈ 440 px).
#[test]
fn widest_legend_fits_the_narrowest_window() {
    use eframe::egui;
    let ctx = egui::Context::default();
    let mut width = 0.0f32;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        let legend = chart::legend(9.99, 9.99, true);
        width = legend
            .iter()
            .map(|t| {
                ctx.fonts(|f| {
                    f.layout_no_wrap(t.clone(), egui::FontId::proportional(11.0), egui::Color32::WHITE)
                        .size()
                        .x
                }) + 26.0
            })
            .sum();
    });
    assert!(width > 0.0 && width < 440.0, "legend width {width}");
}

// ── Mesures de coût (ignorées par défaut) ───────────────────────────────────

fn synthetic_history_json(n: usize) -> String {
    let now = 1_790_000_000u64;
    let v: Vec<serde_json::Value> = (0..n)
        .map(|i| {
            serde_json::json!({"ts": now - 90 * 86_400 + (i as u64) * 35, "pct": 100.0 - (i % 100) as f64,
                               "voltage": 2.9, "schema": 2})
        })
        .collect();
    serde_json::to_string(&v).unwrap()
}

/// Coût, sur le fil d'interface, de `source::load_history` hors D-Bus :
/// décodage du JSON `History(0)` (90 jours à 1 point / 35 s, rythme mesuré
/// dans l'historique réel en avril 2026) puis tri/filtre par trame.
#[test]
#[ignore]
fn measure_history_costs_on_the_ui_thread() {
    for n in [8_640usize, 222_000, 1_000_000] {
        let json = synthetic_history_json(n);
        let t = std::time::Instant::now();
        let entries: Vec<akm_core::history::HistoryEntry> = serde_json::from_str(&json).unwrap();
        let parse = t.elapsed();
        let pts: Vec<(f64, f64)> = entries.iter().map(|e| (e.ts as f64, e.pct)).collect();
        let t = std::time::Instant::now();
        for _ in 0..10 {
            std::hint::black_box(view::chart_points(
                &pts,
                1_790_000_000.0 - 86_400.0,
                86_400.0,
                true,
            ));
        }
        let per_frame = t.elapsed() / 10;
        println!(
            "n={n:>8} json={:>6} KiB parse={parse:?} chart_points/trame={per_frame:?}",
            json.len() / 1024
        );
    }
}

// ── Régressions attendues (échouent tant que le défaut est ouvert) ──────────

/// #236 : un point daté dans le futur (horloge en avance, #166) ne doit pas
/// être tracé comme appartenant aux « dernières 24 h ».
#[test]
fn future_points_are_not_drawn_as_last_24h() {
    let now = 1_790_000_000.0;
    let mut pts: Vec<(f64, f64)> = (0..200).map(|i| (now - 82_800.0 + f64::from(i) * 414.0, 90.0)).collect();
    pts.push((now + 30.0 * 86_400.0, 50.0));
    let out = view::chart_points(&pts, now - 86_400.0, 86_400.0, true);
    assert!(out.iter().all(|p| p.0 <= now + 3_600.0), "future point kept: {:?}", out.last());
}
