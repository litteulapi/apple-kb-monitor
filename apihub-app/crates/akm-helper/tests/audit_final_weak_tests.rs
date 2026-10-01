//! AUDIT FINAL (2026-10-01) — tests que `cargo mutants` a révélés absents :
//! les contrôles « fail closed » de `read_config` (hid-suspend.conf) et la
//! borne de taille de `read_user_file` n'étaient couverts par aucun test
//! (toutes les mutations de `hidctl.rs:1008` et `lib.rs:101` survivaient).
//! Ces tests passent sur le code actuel : ils documentent le comportement
//! attendu et tueraient ces mutants.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use akm_helper::fsutil;
use akm_helper::hidctl;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("akm-audit-weak-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn hid_suspend_conf_not_owned_by_root_is_fail_closed() {
    let d = dir("owner");
    let p = d.join("hid-suspend.conf");
    fs::write(&p, "enabled = true\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    // Owned by the test user (not root): refused whatever its content.
    // SAFETY: geteuid(2) has no failure mode.
    if unsafe { libc::geteuid() } != 0 {
        let e = hidctl::read_config(&p).unwrap_err();
        assert!(e.contains("owned by root"), "{e}");
    }
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hid_suspend_conf_group_or_world_writable_is_fail_closed() {
    let d = dir("mode");
    let p = d.join("hid-suspend.conf");
    fs::write(&p, "enabled = true\n").unwrap();
    for m in [0o666u32, 0o664, 0o622, 0o646] {
        fs::set_permissions(&p, fs::Permissions::from_mode(m)).unwrap();
        assert!(hidctl::read_config(&p).is_err(), "mode {m:o} accepted");
    }
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hid_suspend_conf_larger_than_4k_is_fail_closed() {
    let d = dir("size");
    let p = d.join("hid-suspend.conf");
    let mut s = String::from("enabled = true\n");
    while s.len() <= 4096 {
        s.push_str("# padding\n");
    }
    fs::write(&p, &s).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(hidctl::read_config(&p).is_err());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hid_suspend_conf_symlink_and_absent() {
    let d = dir("link");
    let victim = d.join("victim");
    fs::write(&victim, "enabled = true\n").unwrap();
    let p = d.join("hid-suspend.conf");
    std::os::unix::fs::symlink(&victim, &p).unwrap();
    assert!(hidctl::read_config(&p).is_err(), "symlink followed");
    assert_eq!(hidctl::read_config(&d.join("absent")), Ok(true), "absent = enabled");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn read_user_file_enforces_the_size_bound_exactly() {
    let d = dir("ruf");
    let p = d.join("f");
    // SAFETY: getuid(2) has no failure mode.
    let uid = unsafe { libc::getuid() };
    fs::write(&p, vec![b'a'; 1024]).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(fsutil::read_user_file(&p, uid, 1024).unwrap().len(), 1024);
    fs::write(&p, vec![b'a'; 1025]).unwrap();
    assert!(fsutil::read_user_file(&p, uid, 1024).is_err(), "1025 bytes accepted with max 1024");
    assert!(fsutil::read_regular(&p, 1024).is_err());
    fs::remove_dir_all(&d).unwrap();
}
