//! Settings remembered per keyboard, by address.

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
    /// Offer it in a notification with an "Apply" button (default).
    #[default]
    Ask,
    /// Apply at once (an authentication dialog appears unless the administrator allowed the polkit
    /// action).
    Auto,
}

impl Reapply {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "never" => Some(Self::Off),
            "ask" => Some(Self::Ask),
            "auto" | "always" => Some(Self::Auto),
            _ => None,
        }
    }

    #[must_use]
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

/// Remembered settings of every keyboard (address upper case).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSettings {
    #[serde(default)]
    pub devices: BTreeMap<String, Settings>,
}

impl DeviceSettings {
    /// `$XDG_STATE_HOME/apple-kb-monitor/device-settings.json`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        crate::history::default_path().with_file_name("device-settings.json")
    }

    /// An unreadable or corrupt file is an empty memory.
    #[must_use]
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
    ///
    /// # Errors
    ///
    /// Any I/O error while creating, writing or renaming the file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::fsutil::write_atomic(path, json.as_bytes(), crate::fsutil::Mode::Fixed(0o600))
    }

    /// The user chose Fn mode `mode` while keyboard `mac` was in use.
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

    #[must_use]
    pub fn fn_mode(&self, mac: &str) -> Option<i32> {
        self.devices
            .get(&mac.to_ascii_uppercase())
            .and_then(|s| s.fn_mode)
    }

    pub fn forget(&mut self, mac: &str) -> bool {
        self.devices.remove(&mac.to_ascii_uppercase()).is_some()
    }
}

/// What to do when a keyboard reconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Nothing remembered, already in effect, policy off, or `hid_apple` not loaded.
    Nothing,
    /// Offer to put the remembered mode back.
    Ask(i32),
    /// Put the remembered mode back now.
    Apply(i32),
}

/// `remembered`: the Fn mode kept for the keyboard that reconnects; `live`: the one in effect.
#[must_use]
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
        assert_eq!(plan(Some(2), Some(1), Ask), Plan::Ask(2));
        assert_eq!(plan(Some(1), Some(2), Ask), Plan::Ask(1));
        assert_eq!(plan(Some(2), Some(1), Auto), Plan::Apply(2));
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
        let mut m = DeviceSettings::default();
        for i in 0..40 {
            m.remember_fn_mode(&format!("AA:BB:CC:DD:EE:{i:02X}"), 1);
        }
        assert_eq!(m.devices.len(), MAX_DEVICES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
