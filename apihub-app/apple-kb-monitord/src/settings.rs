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
    /// Where the value in effect is read (and read back after a write, C11).
    sysfs: PathBuf,
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
            sysfs: PathBuf::from(akm_core::hid_params::SYSFS_DIR),
            guard: Arc::default(),
        }
    }

    /// Another parameters directory (tests only).
    pub fn with_sysfs(mut self, dir: impl Into<PathBuf>) -> Self {
        self.sysfs = dir.into();
        self
    }

    fn current(&self, p: Param) -> i32 {
        p.read_in(&self.sysfs)
            .unwrap_or(akm_core::hid_params::UNKNOWN)
    }

    fn acquire(&self) -> Result<(), SetError> {
        let mut g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        if g.running {
            return Err(SetError::Busy(
                "a change of the Fn mode is already waiting for authentication".into(),
            ));
        }
        if let Some(left) = g
            .last_end
            .and_then(|t| COOLDOWN.checked_sub(t.elapsed()))
            .filter(|d| !d.is_zero())
        {
            return Err(SetError::Busy(format!(
                "a change of the Fn mode just ended, retry in {} s",
                left.as_secs().max(1)
            )));
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
        self.current(p)
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
        // Already in effect: no authentication window to change nothing (C11).
        if self.current(p) == v {
            tracing::info!(value = v, "fnmode already in effect, nothing to change");
            return Ok(());
        }
        self.acquire()?;
        tracing::info!(value = v, "fnmode change: opening polkit authentication");
        // Success is the value read back from sysfs, not pkexec's exit code
        // alone: a write that did not happen is never "applied" (C11).
        let r = self.run(v).and_then(|()| match self.current(p) {
            now if now == v => Ok(()),
            now => Err(SetError::Failed(format!(
                "the helper ended without error but the Fn mode in effect is still {now}"
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
        assert_eq!(b.helper, PathBuf::from(HELPER_PATH));
        for p in [Param::SwapOptCmd, Param::IsoLayout] {
            assert!(matches!(b.apply(p, 0), Err(SetError::NotSupported(_))));
        }
        let src = include_str!("settings.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("env::var"), "no env var picks a program");
        assert!(!prod.contains(r#"Command::new("pkexec")"#));
    }

    /// C11: a value already in effect runs nothing; a "success" that did not
    /// change sysfs is a failure; the cooldown says how long to wait.
    #[test]
    fn fnmode_is_checked_against_sysfs() {
        let dir = std::env::temp_dir().join(format!("akm-c11-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fnmode"), "1\n").unwrap();
        let helper = dir.join("akm-helper");
        std::fs::write(&helper, "").unwrap();
        // A "pkexec" that exits 0 and writes nothing, and counts its runs.
        let pk = dir.join("pkexec");
        std::fs::write(
            &pk,
            format!("#!/bin/sh\necho x >> {}/runs\nexit 0\n", dir.display()),
        )
        .unwrap();
        std::fs::set_permissions(&pk, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let b = HelperBackend::with_paths(&pk, &helper).with_sysfs(&dir);
        let runs = || std::fs::read_to_string(dir.join("runs")).map_or(0, |s| s.lines().count());
        assert_eq!(b.get(Param::FnMode), 1);
        b.apply(Param::FnMode, 1).unwrap();
        assert_eq!(runs(), 0, "already in effect: no pkexec");
        let e = b.apply(Param::FnMode, 2).unwrap_err();
        assert_eq!(runs(), 1);
        assert!(matches!(e, SetError::Failed(ref m) if m.contains("still 1")), "{e}");
        let e = b.apply(Param::FnMode, 2).unwrap_err();
        assert!(matches!(e, SetError::Busy(ref m) if m.contains("retry in")), "{e}");
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
