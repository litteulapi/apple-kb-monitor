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
    m.on_event(&Event::Connected("AA:BB:CC:DD:EE:F1".into()), t0);
    assert_eq!(m.due(t0), vec![Action::Acquire]);
    m.acquire_done(true, t0);
    let _ = m.due(t0); // RSSI
    let mut reads = 0;
    // #251 : la 1re lecture planifiee par le modele Apple tombe a 60 s ; la
    // fenetre de Refresh s'arrete juste avant.
    for i in 1..60u64 {
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

/// #208 : sans XDG_RUNTIME_DIR, le verrou n'est plus dans un repertoire fixe
/// partage : nom par uid, 0700, proprietaire verifie, O_NOFOLLOW. Un
/// repertoire/lien prepare par un autre compte est refuse, rien n'est cree.
#[test]
fn lock_fallback_refuses_planted_directory_and_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let base = std::env::temp_dir().join(format!("akm-sec2-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::env::remove_var("XDG_RUNTIME_DIR");
    std::env::set_var("TMPDIR", &base);
    let p = akm_core::read_policy::lock_path();
    let uid = unsafe { libc::getuid() };
    assert_eq!(p, base.join(format!("apple-kb-monitor-{uid}/hid.lock")));
    // Repertoire pre-cree par un tiers : ouvert a tous -> refuse.
    let dir = p.parent().unwrap();
    std::fs::create_dir(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let victim = base.join("fichier-choisi-par-attaquant");
    symlink(&victim, dir.join("hid.lock")).unwrap();
    assert!(akm_core::read_policy::try_lock(Duration::from_millis(50)).is_none());
    assert!(!victim.exists(), "le lien n'a pas ete suivi");
    // Repertoire sain mais lien symbolique pose dedans : O_NOFOLLOW.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(akm_core::read_policy::try_lock(Duration::from_millis(50)).is_none());
    assert!(!victim.exists());
    // Sain : verrou obtenu.
    std::fs::remove_file(dir.join("hid.lock")).unwrap();
    assert!(akm_core::read_policy::try_lock(Duration::from_millis(50)).is_some());
    let _ = std::fs::remove_dir_all(&base);
}
