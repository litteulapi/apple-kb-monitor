//! Per-user base directories: one definition for the daemon, akmctl and the helpers.

use std::path::PathBuf;

/// polkit launcher, absolute: never resolved through `$PATH` (the programs it runs are root).
pub const PKEXEC: &str = "/usr/bin/pkexec";
/// Where the package installs the root helper (also named by the polkit policy and the PKGBUILD).
pub const HELPER: &str = "/usr/lib/apple-kb-monitor/akm-helper";
/// Where the package installs akmctl, run by the daemon for the settings module; absolute like `PKEXEC`.
pub const AKMCTL: &str = "/usr/bin/akmctl";
/// Directory name under every base.
pub const APP_DIR: &str = "apple-kb-monitor";
/// Parent of the per-user runtime directories created by `pam_systemd`.
pub const RUN_USER_ROOT: &str = "/run/user";
/// Base when neither the `XDG_*_HOME` variable nor `HOME` is set: never a shared directory, writes fail.
pub const NO_STATE_HOME: &str = "/nonexistent";

/// Hand-over file of the keymap helper: written by user `uid`, read by the root helper.
#[must_use]
pub fn keymap_source_for_uid(uid: u32) -> PathBuf {
    PathBuf::from(RUN_USER_ROOT)
        .join(uid.to_string())
        .join(APP_DIR)
        .join("keymap.hwdb")
}

type Env<'a> = &'a dyn Fn(&str) -> Option<std::ffi::OsString>;

fn non_empty(env: Env<'_>, var: &str) -> Option<PathBuf> {
    env(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn process_env(var: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(var)
}

/// `$XDG_STATE_HOME`, else `$HOME/.local/state`, else [`NO_STATE_HOME`].
#[must_use]
pub fn state_home() -> PathBuf {
    state_home_in(&process_env)
}

fn state_home_in(env: Env<'_>) -> PathBuf {
    xdg_home_in(env, "XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CONFIG_HOME`, else `$HOME/.config`, else [`NO_STATE_HOME`].
#[must_use]
pub fn config_home() -> PathBuf {
    xdg_home_in(&process_env, "XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME`, else `$HOME/.local/share`, else [`NO_STATE_HOME`].
#[must_use]
pub fn data_home() -> PathBuf {
    xdg_home_in(&process_env, "XDG_DATA_HOME", ".local/share")
}

fn xdg_home_in(env: Env<'_>, var: &str, under_home: &str) -> PathBuf {
    non_empty(env, var)
        .or_else(|| non_empty(env, "HOME").map(|h| h.join(under_home)))
        .unwrap_or_else(|| PathBuf::from(NO_STATE_HOME))
}

/// `<state home>/apple-kb-monitor`.
#[must_use]
pub fn state_dir() -> PathBuf {
    state_home().join(APP_DIR)
}

/// `$XDG_RUNTIME_DIR`, else `/run/user/<uid>`.
#[must_use]
pub fn runtime_home() -> PathBuf {
    runtime_home_in(&process_env)
}

fn runtime_home_in(env: Env<'_>) -> PathBuf {
    non_empty(env, "XDG_RUNTIME_DIR").unwrap_or_else(|| {
        // SAFETY: getuid(2) has no preconditions.
        PathBuf::from(RUN_USER_ROOT).join(unsafe { libc::getuid() }.to_string())
    })
}

/// `<runtime home>/apple-kb-monitor`.
#[must_use]
pub fn runtime_dir() -> PathBuf {
    runtime_home().join(APP_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_hand_over_file() {
        assert_eq!(
            keymap_source_for_uid(1000),
            PathBuf::from("/run/user/1000/apple-kb-monitor/keymap.hwdb")
        );
    }

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.into())
    }

    #[test]
    fn fallbacks_never_use_a_shared_directory() {
        let none = env(&[]);
        assert_eq!(state_home_in(&none), PathBuf::from(NO_STATE_HOME));
        for (var, rel) in [
            ("XDG_CONFIG_HOME", ".config"),
            ("XDG_DATA_HOME", ".local/share"),
        ] {
            assert_eq!(xdg_home_in(&none, var, rel), PathBuf::from(NO_STATE_HOME));
            let vars = [(var, ""), ("HOME", "/h")];
            let home = env(&vars);
            assert_eq!(xdg_home_in(&home, var, rel), PathBuf::from("/h").join(rel));
        }
        // SAFETY: getuid(2) has no preconditions.
        let uid = unsafe { libc::getuid() };
        assert_eq!(
            runtime_home_in(&none),
            PathBuf::from(format!("/run/user/{uid}"))
        );
        let empty = env(&[
            ("XDG_STATE_HOME", ""),
            ("HOME", "/h"),
            ("XDG_RUNTIME_DIR", ""),
        ]);
        assert_eq!(state_home_in(&empty), PathBuf::from("/h/.local/state"));
        assert_eq!(
            runtime_home_in(&empty),
            PathBuf::from(format!("/run/user/{uid}"))
        );
        let set = env(&[
            ("XDG_STATE_HOME", "/s"),
            ("HOME", "/h"),
            ("XDG_RUNTIME_DIR", "/r"),
        ]);
        assert_eq!(state_home_in(&set), PathBuf::from("/s"));
        assert_eq!(runtime_home_in(&set), PathBuf::from("/r"));
    }
}
