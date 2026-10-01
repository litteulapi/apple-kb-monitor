//! Write path of the `hid_apple` parameters (D-Bus `SetFnMode`, #93). The
//! daemon never writes sysfs or `/etc` itself: it validates against the
//! whitelist of [`akm_core::hid_params`] and delegates to the privileged
//! helper (polkit action `com.agenceapi.AppleKbMonitor.set-fnmode`, F07/#88)
//! through a [`SettingsBackend`], replaceable in tests. Only `fnmode` has a
//! polkit action and a helper verb; `swap_opt_cmd`/`iso_layout` are read-only
//! (#203).
//!
//! Hardening (#202/#203): `/usr/bin/pkexec` and the helper are compile-time
//! constants (no `$PATH`, no environment variable picks the program handed to
//! polkit); one authentication dialog at a time, with a cool-down after each
//! attempt, so a session application cannot raise a wall of dialogs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akm_core::hid_params::Param;

/// Where the package installs the helper.
pub const HELPER_PATH: &str = "/usr/lib/apple-kb-monitor/akm-helper";
/// polkit launcher, absolute: never resolved through `$PATH`.
pub const PKEXEC_PATH: &str = "/usr/bin/pkexec";
/// Minimum delay between the end of one authentication attempt and the next.
pub const COOLDOWN: Duration = Duration::from_secs(5);
/// polkit authentication can take a while (password dialog).
const HELPER_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// Value outside the whitelist.
    Invalid(String),
    /// No helper installed.
    NotSupported(String),
    /// Helper refused (polkit denied, write failed...).
    Failed(String),
    /// Another authentication is pending or the previous one ended too
    /// recently.
    Busy(String),
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetError::Invalid(m)
            | SetError::NotSupported(m)
            | SetError::Failed(m)
            | SetError::Busy(m) => {
                f.write_str(m)
            }
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
    /// Current value ([`akm_core::hid_params::UNKNOWN`] if unreadable).
    fn get(&self, p: Param) -> i32;
    /// Apply a value already validated by [`validate`].
    fn apply(&self, p: Param, v: i32) -> Result<(), SetError>;
}

/// Whitelist check shared by every backend.
pub fn validate(p: Param, v: i32) -> Result<(), SetError> {
    if p.valid(v) {
        Ok(())
    } else {
        Err(SetError::Invalid(format!(
            "{v} is not a valid {}",
            p.name()
        )))
    }
}

/// Validate then apply.
pub fn set(backend: &dyn SettingsBackend, p: Param, v: i32) -> Result<(), SetError> {
    validate(p, v)?;
    backend.apply(p, v)
}

/// Single-flight + cool-down state shared by the clones of a backend.
#[derive(Debug, Default)]
struct Guard {
    running: bool,
    last_end: Option<Instant>,
}

/// Real backend: sysfs for reading, `/usr/bin/pkexec <helper> set-fnmode
/// <value>` for writing.
#[derive(Debug, Clone)]
pub struct HelperBackend {
    pkexec: PathBuf,
    helper: PathBuf,
    guard: Arc<Mutex<Guard>>,
}

impl Default for HelperBackend {
    fn default() -> Self {
        Self::with_paths(PKEXEC_PATH, HELPER_PATH)
    }
}

impl HelperBackend {
    /// Explicit programs (tests only: the daemon uses [`Default`], whose paths
    /// are constants).
    pub fn with_paths(pkexec: impl Into<PathBuf>, helper: impl Into<PathBuf>) -> Self {
        Self {
            pkexec: pkexec.into(),
            helper: helper.into(),
            guard: Arc::default(),
        }
    }

    fn acquire(&self) -> Result<(), SetError> {
        let mut g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        if g.running {
            return Err(SetError::Busy("an authentication is already pending".into()));
        }
        if g.last_end.is_some_and(|t| t.elapsed() < COOLDOWN) {
            return Err(SetError::Busy("too many requests, retry shortly".into()));
        }
        g.running = true;
        Ok(())
    }

    fn release(&self) {
        let mut g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        g.running = false;
        g.last_end = Some(Instant::now());
    }

    fn run(&self, v: i32) -> Result<(), SetError> {
        let mut child = Command::new(&self.pkexec)
            .arg(&self.helper)
            .args(["set-fnmode", &v.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SetError::Failed(format!("cannot run pkexec: {e}")))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => return Ok(()),
                Ok(Some(st)) => {
                    let mut msg = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        let _ = std::io::Read::read_to_string(&mut e, &mut msg);
                    }
                    return Err(SetError::Failed(format!(
                        "helper failed ({st}): {}",
                        msg.trim()
                    )));
                }
                Ok(None) if start.elapsed() > HELPER_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SetError::Failed("helper timed out".into()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(SetError::Failed(e.to_string())),
            }
        }
    }
}

impl SettingsBackend for HelperBackend {
    fn get(&self, p: Param) -> i32 {
        p.read()
    }

    fn apply(&self, p: Param, v: i32) -> Result<(), SetError> {
        if p != Param::FnMode {
            return Err(SetError::NotSupported(format!(
                "{} cannot be changed (read-only)",
                p.name()
            )));
        }
        if !self.helper.is_file() {
            return Err(SetError::NotSupported(format!(
                "privileged helper not installed ({})",
                self.helper.display()
            )));
        }
        self.acquire()?;
        tracing::info!(value = v, "fnmode change: opening polkit authentication");
        let r = self.run(v);
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
        assert_eq!(b.helper, PathBuf::from(HELPER_PATH));
        for p in [Param::SwapOptCmd, Param::IsoLayout] {
            assert!(matches!(b.apply(p, 0), Err(SetError::NotSupported(_))));
        }
        let src = include_str!("settings.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("env::var"), "no env var picks a program");
        assert!(!prod.contains(r#"Command::new("pkexec")"#));
    }

    #[test]
    fn errors_map_to_dbus_errors() {
        let e: zbus::fdo::Error = SetError::NotSupported("x".into()).into();
        assert!(matches!(e, zbus::fdo::Error::NotSupported(_)));
        let e: zbus::fdo::Error = SetError::Invalid("x".into()).into();
        assert!(matches!(e, zbus::fdo::Error::InvalidArgs(_)));
    }
}
