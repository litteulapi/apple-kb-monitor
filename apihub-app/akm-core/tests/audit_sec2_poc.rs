//! Audit securite 2 (docs/AUDIT-SECURITE-2.md) : preuves INOFFENSIVES, sans
//! materiel (machine d'etats pure, repertoires temporaires).
//!
//! `cargo test -p akm-core --test audit_sec2_poc -- --nocapture --test-threads=1`

use std::time::{Duration, Instant};

use akm_core::machine::{Action, Event, Machine};

/// PoC-D : `Refresh()` (D-Bus, appelable par toute appli de la session) remet
/// la lecture HID lente a "maintenant" sans espacement minimal : 1 appel par
/// seconde = 1 salve GET_REPORT par seconde au lieu d'une toutes les 4 h (#177).
#[test]
fn poc_refresh_defeats_slow_read_period() {
    let t0 = Instant::now();
    let mut m = Machine::new();
    m.on_event(&Event::Connected("04:DB:56:CA:42:EE".into()), t0);
    assert_eq!(m.due(t0), vec![Action::Acquire]);
    m.acquire_done(true, t0);
    let _ = m.due(t0); // RSSI
    let mut reads = 0;
    for i in 1..=60u64 {
        let t = t0 + Duration::from_secs(i);
        m.force_refresh(t); // = un appel D-Bus Refresh()
        if m.due(t).contains(&Action::Acquire) {
            reads += 1;
            m.acquire_done(true, t);
        }
    }
    println!("POC-D lectures HID declenchees en 60 s par Refresh() : {reads} (attendu sans Refresh : 0, periode 4 h)");
    assert_eq!(reads, 60);
}

/// PoC-E : sans XDG_RUNTIME_DIR (sudo, ssh sans logind, cron), le verrou est
/// `$TMPDIR|/tmp/apple-kb-monitor/hid.lock` : chemin partage entre comptes,
/// ouvert en O_CREAT sans O_NOFOLLOW. Un repertoire prepare par un autre compte
/// avec un lien symbolique fait creer un fichier choisi par lui. Simule dans un
/// TMPDIR prive (aucun /tmp reel touche).
#[test]
fn poc_lock_fallback_follows_symlink() {
    let base = std::env::temp_dir().join(format!("akm-sec2-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("apple-kb-monitor")).unwrap();
    let victim = base.join("fichier-choisi-par-attaquant");
    std::os::unix::fs::symlink(&victim, base.join("apple-kb-monitor/hid.lock")).unwrap();
    std::env::remove_var("XDG_RUNTIME_DIR");
    std::env::set_var("TMPDIR", &base);
    let p = akm_core::read_policy::lock_path();
    println!("POC-E lock_path() sans XDG_RUNTIME_DIR = {}", p.display());
    assert_eq!(p, base.join("apple-kb-monitor/hid.lock"));
    let _l = akm_core::read_policy::try_lock(Duration::from_millis(50));
    println!("POC-E cible du lien creee : {}", victim.exists());
    assert!(victim.exists(), "le lien symbolique a ete suivi");
    let _ = std::fs::remove_dir_all(&base);
}
