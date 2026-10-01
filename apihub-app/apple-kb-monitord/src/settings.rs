//! Write path of the `hid_apple` parameters (D-Bus `SetFnMode`,
//! `SetSwapOptCmd`, `SetIsoLayout`, #93). The daemon never writes sysfs or
//! `/etc` itself: it validates against the whitelist of
//! [`akm_core::hid_params`] and delegates to the privileged helper (polkit
//! action `com.agenceapi.AppleKbMonitor.configure`, F07/#88) through a
//! [`SettingsBackend`], replaceable in tests.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use akm_core::hid_params::Param;

/// Where the package installs the helper.
pub const HELPER_PATH: &str = "/usr/lib/apple-kb-monitor/akm-helper";
/// Development override of [`HELPER_PATH`].
pub const HELPER_ENV: &str = "APPLE_KB_SETTINGS_HELPER";
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
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetError::Invalid(m) | SetError::NotSupported(m) | SetError::Failed(m) => {
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

/// Real backend: sysfs for reading, `pkexec <helper> set <name> <value>`
/// for writing.
#[derive(Debug, Clone, Default)]
pub struct HelperBackend;

impl HelperBackend {
    fn helper() -> PathBuf {
        std::env::var_os(HELPER_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(HELPER_PATH))
    }
}

impl SettingsBackend for HelperBackend {
    fn get(&self, p: Param) -> i32 {
        p.read()
    }

    fn apply(&self, p: Param, v: i32) -> Result<(), SetError> {
        let helper = Self::helper();
        if !helper.is_file() {
            return Err(SetError::NotSupported(format!(
                "privileged helper not installed ({})",
                helper.display()
            )));
        }
        let mut child = Command::new("pkexec")
            .arg(&helper)
            .args(["set", p.name(), &v.to_string()])
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
    fn errors_map_to_dbus_errors() {
        let e: zbus::fdo::Error = SetError::NotSupported("x".into()).into();
        assert!(matches!(e, zbus::fdo::Error::NotSupported(_)));
        let e: zbus::fdo::Error = SetError::Invalid("x".into()).into();
        assert!(matches!(e, zbus::fdo::Error::InvalidArgs(_)));
    }
}
