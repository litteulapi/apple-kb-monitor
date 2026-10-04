//! Write path of the `hid_apple` parameters (D-Bus `SetFnMode`).

use std::path::PathBuf;
use std::time::Duration;

use akm_core::hid_params::Param;
use akm_core::paths::{HELPER, PKEXEC};

use crate::privileged::{self, Busy, Flight};

/// Minimum delay between the end of one authentication attempt and the next.
pub const COOLDOWN: Duration = Duration::from_secs(5);

/// D-Bus texts are translated here; the tray recognises them through these helpers.
#[must_use]
pub fn pending_text() -> String {
    tr!("a change of the Fn mode is already waiting for authentication")
}

#[must_use]
pub fn cooldown_text(secs: u64) -> String {
    tr!(
        "a change of the Fn mode just ended, retry in {s} s",
        s = secs
    )
}

/// Seconds left in a [`cooldown_text`], in the daemon's language.
#[must_use]
pub fn cooldown_secs(msg: &str) -> Option<String> {
    let t = akm_core::i18n::gettext("a change of the Fn mode just ended, retry in {s} s");
    let (pre, post) = t.split_once("{s}")?;
    let n = msg.strip_prefix(pre)?.strip_suffix(post)?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// Value outside the whitelist.
    Invalid(String),
    /// No helper installed.
    NotSupported(String),
    /// Helper refused (polkit denied, write failed...).
    Failed(String),
    /// Another authentication is pending or the previous one ended too recently.
    Busy(String),
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetError::Invalid(m)
            | SetError::NotSupported(m)
            | SetError::Failed(m)
            | SetError::Busy(m) => f.write_str(m),
        }
    }
}

impl From<SetError> for zbus::fdo::Error {
    fn from(e: SetError) -> Self {
        match e {
            SetError::Invalid(m) => zbus::fdo::Error::InvalidArgs(m),
            SetError::NotSupported(m) => zbus::fdo::Error::NotSupported(m),
            SetError::Failed(m) => zbus::fdo::Error::Failed(m),
            SetError::Busy(m) => zbus::fdo::Error::LimitsExceeded(m),
        }
    }
}

/// Reads and writes `hid_apple` parameters.
pub trait SettingsBackend: Send + Sync {
    fn get(&self, p: Param) -> i32;
    /// # Errors
    /// [`SetError`] when the write is refused or fails.
    fn apply(&self, p: Param, v: i32) -> Result<(), SetError>;
}

/// Whitelist check shared by every backend.
///
/// # Errors
/// [`SetError`] when the parameter or the value is not allowed.
pub fn validate(p: Param, v: i32) -> Result<(), SetError> {
    if p.valid(v) {
        Ok(())
    } else {
        Err(SetError::Invalid(tr!(
            "{v} is not a valid {param}",
            v = v,
            param = p.name()
        )))
    }
}

/// Validate then apply.
///
/// # Errors
/// [`SetError`] when the value is refused or the write fails.
pub fn set(backend: &dyn SettingsBackend, p: Param, v: i32) -> Result<(), SetError> {
    validate(p, v)?;
    backend.apply(p, v)
}

/// Real backend: sysfs for reading, `/usr/bin/pkexec <helper> set-fnmode <value>` for writing.
#[derive(Debug, Clone)]
pub struct HelperBackend {
    pkexec: PathBuf,
    helper: PathBuf,
    sysfs: PathBuf,
    guard: Flight,
}

impl Default for HelperBackend {
    fn default() -> Self {
        Self {
            guard: Flight::shared(),
            ..Self::with_paths(PKEXEC, HELPER)
        }
    }
}

impl HelperBackend {
    /// Explicit programs (tests only: the daemon uses [`Default`], whose paths are constants).
    pub fn with_paths(pkexec: impl Into<PathBuf>, helper: impl Into<PathBuf>) -> Self {
        Self {
            pkexec: pkexec.into(),
            helper: helper.into(),
            sysfs: PathBuf::from(akm_core::hid_params::SYSFS_DIR),
            guard: Flight::default(),
        }
    }

    #[cfg(test)]
    /// Another parameters directory (tests only).
    #[must_use]
    pub fn with_sysfs(mut self, dir: impl Into<PathBuf>) -> Self {
        self.sysfs = dir.into();
        self
    }

    fn current(&self, p: Param) -> i32 {
        p.read_in(&self.sysfs)
            .unwrap_or(akm_core::hid_params::UNKNOWN)
    }

    fn acquire(&self) -> Result<(), SetError> {
        self.guard.acquire().map_err(|b| {
            SetError::Busy(match b {
                Busy::Pending => pending_text(),
                Busy::CoolDown(left) => cooldown_text(left.as_secs().max(1)),
            })
        })
    }

    fn release(&self) {
        self.guard.release();
    }

    fn run(&self, v: i32) -> Result<(), SetError> {
        privileged::run(
            &self.pkexec,
            &self.helper,
            &["set-fnmode".into(), v.to_string()],
        )
        .map_err(SetError::Failed)
    }
}

impl SettingsBackend for HelperBackend {
    fn get(&self, p: Param) -> i32 {
        self.current(p)
    }

    fn apply(&self, p: Param, v: i32) -> Result<(), SetError> {
        if p != Param::FnMode {
            return Err(SetError::NotSupported(tr!(
                "{name} cannot be changed (read-only)",
                name = p.name()
            )));
        }
        if !self.helper.is_file() {
            return Err(SetError::NotSupported(tr!(
                "privileged helper not installed ({program})",
                program = self.helper.display()
            )));
        }
        // Already in effect: no authentication window to change nothing.
        if self.current(p) == v {
            tracing::info!(value = v, "fnmode already in effect, nothing to change");
            return Ok(());
        }
        self.acquire()?;
        tracing::info!(value = v, "fnmode change: opening polkit authentication");
        // Success is the value read back from sysfs, not pkexec's exit code alone.
        let r = self.run(v).and_then(|()| match self.current(p) {
            now if now == v => Ok(()),
            now => Err(SetError::Failed(tr!(
                "the helper ended without error but the Fn mode in effect is still {now}",
                now = now
            ))),
        });
        self.release();
        match &r {
            Ok(()) => tracing::info!(value = v, "fnmode change applied"),
            Err(e) => tracing::warn!(value = v, error = %e, "fnmode change failed"),
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake(Mutex<Vec<(Param, i32)>>);
    impl SettingsBackend for Fake {
        fn get(&self, _: Param) -> i32 {
            1
        }
        fn apply(&self, p: Param, v: i32) -> Result<(), SetError> {
            self.0.lock().unwrap().push((p, v));
            Ok(())
        }
    }

    #[test]
    fn invalid_values_never_reach_the_backend() {
        let f = Fake::default();
        assert!(matches!(
            set(&f, Param::FnMode, 7),
            Err(SetError::Invalid(_))
        ));
        assert!(matches!(
            set(&f, Param::IsoLayout, -2),
            Err(SetError::Invalid(_))
        ));
        set(&f, Param::FnMode, 2).unwrap();
        set(&f, Param::IsoLayout, -1).unwrap();
        assert_eq!(
            *f.0.lock().unwrap(),
            vec![(Param::FnMode, 2), (Param::IsoLayout, -1)]
        );
    }

    #[test]
    fn only_fnmode_is_writable_and_programs_are_constants() {
        let b = HelperBackend::default();
        assert_eq!(b.pkexec, PathBuf::from("/usr/bin/pkexec"));
        assert_eq!(b.helper, PathBuf::from(HELPER));
        for p in [Param::SwapOptCmd, Param::IsoLayout] {
            assert!(matches!(b.apply(p, 0), Err(SetError::NotSupported(_))));
        }
        let prod = akm_core::srclint::prod_tokens(include_str!("settings.rs"));
        assert!(!prod.contains("env::var"), "no env var picks a program");
        assert!(!prod.contains(r#"Command::new("pkexec")"#));
    }

    #[test]
    fn fnmode_is_checked_against_sysfs() {
        let dir = std::env::temp_dir().join(format!("akm-c11-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fnmode"), "1\n").unwrap();
        let helper = dir.join("akm-helper");
        std::fs::write(&helper, "").unwrap();
        let pk = dir.join("pkexec");
        std::fs::write(
            &pk,
            format!("#!/bin/sh\necho x >> {}/runs\nexit 0\n", dir.display()),
        )
        .unwrap();
        std::fs::set_permissions(&pk, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let b = HelperBackend::with_paths(&pk, &helper).with_sysfs(&dir);
        let runs = || std::fs::read_to_string(dir.join("runs")).map_or(0, |s| s.lines().count());
        assert_eq!(b.get(Param::FnMode), 1);
        b.apply(Param::FnMode, 1).unwrap();
        assert_eq!(runs(), 0, "already in effect: no pkexec");
        let e = b.apply(Param::FnMode, 2).unwrap_err();
        assert_eq!(runs(), 1);
        assert!(
            matches!(e, SetError::Failed(ref m) if m.contains("still 1")),
            "{e}"
        );
        let e = b.apply(Param::FnMode, 2).unwrap_err();
        assert!(
            matches!(e, SetError::Busy(ref m) if m.contains("retry in")),
            "{e}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_map_to_dbus_errors() {
        let e: zbus::fdo::Error = SetError::NotSupported("x".into()).into();
        assert!(matches!(e, zbus::fdo::Error::NotSupported(_)));
        let e: zbus::fdo::Error = SetError::Invalid("x".into()).into();
        assert!(matches!(e, zbus::fdo::Error::InvalidArgs(_)));
    }
}
