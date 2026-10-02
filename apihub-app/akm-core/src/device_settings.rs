//! Settings remembered per keyboard, by address (#103).
//!
//! `hid_apple` has ONE set of parameters for every Apple keyboard of the
//! computer. What can be done per keyboard is to remember what the user chose
//! while that keyboard was the one in use, and to put it back when that
//! keyboard reconnects. This module is the memory and the decision; it never
//! writes anything itself.
//!
//! * [`DeviceSettings`]: address -> remembered Fn mode, persisted, bounded.
//! * [`plan`]: at a reconnection, compare what is remembered with what is in
//!   effect and say what to do under the configured [`Reapply`] policy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hid_params::Param;

/// Keyboards remembered (the alphabetically last address goes first beyond).
pub const MAX_DEVICES: usize = 16;

/// What to do at a reconnection (`[devices] reapply_settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reapply {
    /// Nothing: the settings in effect stay.
    Off,
    /// Offer it in a notification with an "Apply" button (default): changing
    /// a `hid_apple` parameter asks for an authentication, never raised
    /// without the user asking.
    #[default]
    Ask,
    /// Apply at once (an authentication dialog appears unless the
    /// administrator allowed the polkit action).
    Auto,
}

impl Reapply {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "never" => Some(Self::Off),
            "ask" => Some(Self::Ask),
            "auto" | "always" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }
}

/// What is remembered for one keyboard.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// `hid_apple.fnmode` chosen while this keyboard was in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fn_mode: Option<i32>,
}

/// Remembered settings of every keyboard (address upper case). Persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSettings {
    #[serde(default)]
    pub devices: BTreeMap<String, Settings>,
}

impl DeviceSettings {
    /// `$XDG_STATE_HOME/apple-kb-monitor/device-settings.json`.
    pub fn default_path() -> PathBuf {
        crate::history::default_path().with_file_name("device-settings.json")
    }

    /// An unreadable or corrupt file is an empty memory; a value outside the
    /// whitelist of [`Param`] is dropped (it would never be applied).
    pub fn load(path: &Path) -> Self {
        let mut s: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        for v in s.devices.values_mut() {
            v.fn_mode = v.fn_mode.filter(|m| Param::FnMode.valid(*m));
        }
        s.devices.retain(|_, v| *v != Settings::default());
        while s.devices.len() > MAX_DEVICES {
            s.devices.pop_last();
        }
        s
    }

    /// Atomic write (temporary file + rename), mode 0600.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(
            serde_json::to_string_pretty(self)
                .map_err(std::io::Error::other)?
                .as_bytes(),
        )?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    }

    /// The user chose Fn mode `mode` while keyboard `mac` was in use.
    /// Returns whether the memory changed. An invalid mode is refused.
    pub fn remember_fn_mode(&mut self, mac: &str, mode: i32) -> bool {
        if !Param::FnMode.valid(mode) {
            return false;
        }
        let key = mac.to_ascii_uppercase();
        if !self.devices.contains_key(&key) && self.devices.len() >= MAX_DEVICES {
            self.devices.pop_last();
        }
        let s = self.devices.entry(key).or_default();
        let changed = s.fn_mode != Some(mode);
        s.fn_mode = Some(mode);
        changed
    }

    /// The Fn mode remembered for `mac`.
    pub fn fn_mode(&self, mac: &str) -> Option<i32> {
        self.devices
            .get(&mac.to_ascii_uppercase())
            .and_then(|s| s.fn_mode)
    }

    /// Forget what is remembered for `mac`.
    pub fn forget(&mut self, mac: &str) -> bool {
        self.devices.remove(&mac.to_ascii_uppercase()).is_some()
    }
}

/// What to do when a keyboard reconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Nothing remembered, already in effect, policy off, or `hid_apple` not
    /// loaded (nothing to compare with).
    Nothing,
    /// Offer to put the remembered mode back.
    Ask(i32),
    /// Put the remembered mode back now.
    Apply(i32),
}

/// `remembered`: the Fn mode kept for the keyboard that reconnects; `live`:
/// the one in effect (`None` = `hid_apple` not loaded).
pub fn plan(remembered: Option<i32>, live: Option<i32>, policy: Reapply) -> Plan {
    match (remembered, live, policy) {
        (Some(want), Some(now), Reapply::Ask) if want != now => Plan::Ask(want),
        (Some(want), Some(now), Reapply::Auto) if want != now => Plan::Apply(want),
        _ => Plan::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "AA:BB:CC:DD:EE:F1";
    const B: &str = "AA:BB:CC:DD:EE:F2";

    #[test]
    fn each_keyboard_remembers_its_own_mode() {
        let mut m = DeviceSettings::default();
        assert_eq!(m.fn_mode(A), None);
        assert!(m.remember_fn_mode(A, 2));
        assert!(m.remember_fn_mode("aa:bb:cc:dd:ee:f2", 1));
        assert!(!m.remember_fn_mode(A, 2), "unchanged");
        assert_eq!((m.fn_mode(A), m.fn_mode(B)), (Some(2), Some(1)));
        assert!(m.remember_fn_mode(A, 1));
        assert_eq!(m.fn_mode("aa:bb:cc:dd:ee:f1"), Some(1));
        assert!(!m.remember_fn_mode(A, 9), "outside the whitelist");
        assert_eq!(m.fn_mode(A), Some(1));
        assert!(m.forget(A) && !m.forget(A));
        assert_eq!(m.fn_mode(A), None);
    }

    #[test]
    fn the_plan_follows_the_policy() {
        use Reapply::{Ask, Auto, Off};
        // Two keyboards with different modes: each gets its own back.
        assert_eq!(plan(Some(2), Some(1), Ask), Plan::Ask(2));
        assert_eq!(plan(Some(1), Some(2), Ask), Plan::Ask(1));
        assert_eq!(plan(Some(2), Some(1), Auto), Plan::Apply(2));
        // Already in effect, nothing remembered, policy off, module absent.
        assert_eq!(plan(Some(2), Some(2), Auto), Plan::Nothing);
        assert_eq!(plan(None, Some(1), Auto), Plan::Nothing);
        assert_eq!(plan(Some(2), Some(1), Off), Plan::Nothing);
        assert_eq!(plan(Some(2), None, Auto), Plan::Nothing);
        assert_eq!(
            Reapply::default(),
            Ask,
            "never an authentication dialog out of the blue"
        );
    }

    #[test]
    fn policy_names() {
        for (s, p) in [
            ("ask", Reapply::Ask),
            (" AUTO ", Reapply::Auto),
            ("off", Reapply::Off),
            ("never", Reapply::Off),
            ("always", Reapply::Auto),
        ] {
            assert_eq!(Reapply::parse(s), Some(p), "{s}");
        }
        assert_eq!(Reapply::parse("yes"), None);
        for p in [Reapply::Off, Reapply::Ask, Reapply::Auto] {
            assert_eq!(Reapply::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn the_memory_survives_a_restart_and_drops_what_cannot_be_applied() {
        let dir = std::env::temp_dir().join(format!("akm-devset-{}", std::process::id()));
        let path = dir.join("device-settings.json");
        let mut m = DeviceSettings::default();
        m.remember_fn_mode(A, 2);
        m.remember_fn_mode(B, 1);
        m.save(&path).unwrap();
        assert_eq!(DeviceSettings::load(&path), m);
        std::fs::write(
            &path,
            r#"{"devices":{"AA:BB:CC:DD:EE:F1":{"fn_mode":7},"AA:BB:CC:DD:EE:F2":{"fn_mode":1}}}"#,
        )
        .unwrap();
        let back = DeviceSettings::load(&path);
        assert_eq!((back.fn_mode(A), back.fn_mode(B)), (None, Some(1)));
        assert_eq!(back.devices.len(), 1);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(DeviceSettings::load(&path), DeviceSettings::default());
        // Bounded.
        let mut m = DeviceSettings::default();
        for i in 0..40 {
            m.remember_fn_mode(&format!("AA:BB:CC:DD:EE:{i:02X}"), 1);
        }
        assert_eq!(m.devices.len(), MAX_DEVICES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
