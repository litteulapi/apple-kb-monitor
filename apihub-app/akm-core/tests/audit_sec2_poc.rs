//! Security tests: pure state machine, temporary directories.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use akm_core::machine::{Action, Event, Machine, RefreshOutcome, FORCE_REFRESH_FLOOR};

#[test]
fn refresh_cannot_defeat_slow_read_period() {
    let t0 = Instant::now();
    let mut m = Machine::new();
    m.on_event(&Event::Connected("AA:BB:CC:DD:EE:F1".into()), t0);
    assert_eq!(m.due(t0), vec![Action::Acquire]);
    m.acquire_done(true, t0);
    let _ = m.due(t0); // RSSI
    let mut reads = 0;
    for i in 1..60u64 {
        let t = t0 + Duration::from_secs(i);
        let _ = m.request_refresh(t);
        if m.due(t).contains(&Action::Acquire) {
            reads += 1;
            m.acquire_done(true, t);
        }
    }
    assert_eq!(reads, 0);
    let t = t0 + FORCE_REFRESH_FLOOR + Duration::from_secs(1);
    assert_eq!(m.request_refresh(t), RefreshOutcome::Accepted);
    assert!(m.due(t).contains(&Action::Acquire));
    m.acquire_done(true, t);
    for i in 1..=60u64 {
        let t = t + Duration::from_secs(i);
        let _ = m.request_refresh(t);
        assert!(!m.due(t).contains(&Action::Acquire));
    }
}

#[test]
fn lock_fallback_refuses_planted_directory_and_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let base = std::env::temp_dir().join(format!("akm-sec2-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::env::remove_var("XDG_RUNTIME_DIR");
    let uid = unsafe { libc::getuid() };
    let run_user = format!("/run/user/{uid}/apple-kb-monitor/hid.lock");
    assert_eq!(
        akm_core::read_policy::lock_path(),
        PathBuf::from(run_user),
        "never TMPDIR"
    );
    std::env::set_var("XDG_RUNTIME_DIR", &base);
    let p = akm_core::read_policy::lock_path();
    assert_eq!(p, base.join("apple-kb-monitor/hid.lock"));
    let dir = p.parent().unwrap();
    std::fs::create_dir(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let victim = base.join("file-chosen-by-attacker");
    symlink(&victim, dir.join("hid.lock")).unwrap();
    assert!(akm_core::read_policy::try_lock(Duration::from_millis(50)).is_none());
    assert!(!victim.exists(), "the link was not followed");
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(akm_core::read_policy::try_lock(Duration::from_millis(50)).is_none());
    assert!(!victim.exists());
    std::fs::remove_file(dir.join("hid.lock")).unwrap();
    assert!(akm_core::read_policy::try_lock(Duration::from_millis(50)).is_some());
    let _ = std::fs::remove_dir_all(&base);
}
