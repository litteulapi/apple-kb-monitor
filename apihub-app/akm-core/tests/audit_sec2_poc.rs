//! Regressions de l'audit securite 2 (docs/AUDIT-SECURITE-2.md), preuves
//! d'origine inversees : machine d'etats pure, repertoires temporaires.

use std::time::{Duration, Instant};

use akm_core::machine::{Action, Event, Machine, FORCE_REFRESH_FLOOR};

/// #206 : `Refresh()` repete (1/s pendant 60 s) ne declenche AUCUNE lecture HID
/// (avant : 60). Un Refresh apres le plancher de 5 min est honore, une seule fois.
#[test]
fn refresh_cannot_defeat_slow_read_period() {
    let t0 = Instant::now();
    let mut m = Machine::new();
    m.on_event(&Event::Connected("04:DB:56:CA:42:EE".into()), t0);
    assert_eq!(m.due(t0), vec![Action::Acquire]);
    m.acquire_done(true, t0);
    let _ = m.due(t0); // RSSI
    let mut reads = 0;
    for i in 1..=60u64 {
        let t = t0 + Duration::from_secs(i);
        m.force_refresh(t);
        if m.due(t).contains(&Action::Acquire) {
            reads += 1;
            m.acquire_done(true, t);
        }
    }
    assert_eq!(reads, 0);
    // Apres le plancher, un Refresh explicite est honore une fois, puis ignore.
    let t = t0 + FORCE_REFRESH_FLOOR + Duration::from_secs(1);
    assert!(m.force_refresh(t));
    assert!(m.due(t).contains(&Action::Acquire));
    m.acquire_done(true, t);
    for i in 1..=60u64 {
        let t = t + Duration::from_secs(i);
        m.force_refresh(t);
        assert!(!m.due(t).contains(&Action::Acquire));
    }
}
