//! "Fail closed" checks of `read_config` and the size bound of `read_user_file`, found
//! untested by `cargo mutants`.

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

fn is_root() -> bool {
    // SAFETY: geteuid(2) has no failure mode.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn hid_suspend_conf_not_owned_by_root_is_fail_closed() {
    let d = dir("owner");
    let p = d.join("hid-suspend.conf");
    fs::write(&p, "enabled = true\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    if !is_root() {
        let e = hidctl::read_config(&p).unwrap_err();
        assert!(e.contains("owned by root"), "{e}");
    }
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hid_suspend_conf_group_or_world_writable_is_fail_closed() {
    // As non-root the file is refused for its owner before mode or size is looked at.
    if !is_root() {
        return;
    }
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
    // As non-root the file is refused for its owner before mode or size is looked at.
    if !is_root() {
        return;
    }
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
    assert_eq!(
        hidctl::read_config(&d.join("absent")),
        Ok(false),
        "absent = disabled (SUSPEND is opt-in)"
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hid_suspend_conf_metadata_checks_one_by_one() {
    use hidctl::{config_meta_ok, CONFIG_MAX_LEN};
    assert_eq!(CONFIG_MAX_LEN, 4096);
    assert!(config_meta_ok(true, 0, 0o100_644, 0));
    assert!(
        config_meta_ok(true, 0, 0o100_644, 4096),
        "exactly 4 KiB accepted"
    );
    assert!(config_meta_ok(true, 0, 0o100_600, 12));
    assert!(
        config_meta_ok(true, 0, 0o100_755, 12),
        "executable bits do not matter"
    );
    assert!(
        !config_meta_ok(true, 0, 0o100_644, 4097),
        "4 KiB + 1 refused"
    );
    assert!(
        !config_meta_ok(false, 0, 0o100_644, 12),
        "not a regular file"
    );
    assert!(!config_meta_ok(true, 1, 0o100_644, 12), "not owned by root");
    assert!(
        !config_meta_ok(true, 1000, 0o100_644, 12),
        "not owned by root"
    );
    assert!(!config_meta_ok(true, 0, 0o100_664, 12), "group writable");
    assert!(!config_meta_ok(true, 0, 0o100_646, 12), "world writable");
    assert!(
        !config_meta_ok(true, 0, 0o100_620, 12),
        "group writable only"
    );
    assert!(
        !config_meta_ok(true, 0, 0o100_602, 12),
        "world writable only"
    );
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
    assert!(
        fsutil::read_user_file(&p, uid, 1024).is_err(),
        "1025 bytes accepted with max 1024"
    );
    assert!(fsutil::read_regular(&p, 1024).is_err());
    fs::remove_dir_all(&d).unwrap();
}
