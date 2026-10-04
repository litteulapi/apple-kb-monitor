//! Command line of the daemon binary: rejected before any bus or keyboard access.

use std::os::unix::ffi::OsStringExt;
use std::process::Command;

#[test]
fn a_non_utf8_argument_exits_with_the_usage_code() {
    let tmp = std::env::temp_dir().join(format!("akm-cli-args-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_apple-kb-monitord"))
        .arg(std::ffi::OsString::from_vec(vec![b'-', 0xff]))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/akm-test-bus",
        )
        .env("XDG_RUNTIME_DIR", &tmp)
        .env("XDG_STATE_HOME", &tmp)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not UTF-8"));
}
