//! `hid_apple` module parameters (`/sys/module/hid_apple/parameters`).

use crate::tr;
use std::path::Path;

pub const SYSFS_DIR: &str = "/sys/module/hid_apple/parameters";
/// D-Bus value of a parameter that cannot be read (module not loaded).
pub const UNKNOWN: i32 = -100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    /// 0 disabled, 1 media keys first (Fn = F-keys), 2 F-keys first, 3 auto.
    FnMode,
    /// 0 normal, 1 swap Option/Command, 2 swap left side only.
    SwapOptCmd,
    /// -1 auto, 0 ANSI, 1 ISO.
    IsoLayout,
}

impl Param {
    pub const ALL: [Param; 3] = [Param::FnMode, Param::SwapOptCmd, Param::IsoLayout];

    /// sysfs / modprobe name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Param::FnMode => "fnmode",
            Param::SwapOptCmd => "swap_opt_cmd",
            Param::IsoLayout => "iso_layout",
        }
    }

    /// In the range `modinfo hid_apple` documents ([`crate::keymap::KERNEL_PARAMS`]).
    #[must_use]
    pub fn valid(self, v: i32) -> bool {
        crate::keymap::kernel_param(self.name()).is_some_and(|k| (k.min..=k.max).contains(&v))
    }

    /// Current value under `dir` (`None` if absent or unreadable).
    #[must_use]
    pub fn read_in(self, dir: &Path) -> Option<i32> {
        std::fs::read_to_string(dir.join(self.name()))
            .ok()?
            .trim()
            .parse()
            .ok()
            .filter(|v| self.valid(*v))
    }

    /// Current value, [`UNKNOWN`] if it cannot be read.
    #[must_use]
    pub fn read(self) -> i32 {
        self.read_in(Path::new(SYSFS_DIR)).unwrap_or(UNKNOWN)
    }
}

/// What an `hid_apple.fnmode` value does: short wording (OSD, menu) and long wording (`akmctl`).
fn fn_mode_words(mode: i32) -> Option<(String, String)> {
    Some(match mode {
        0 => (
            tr!("Fn key has no effect"),
            tr!("disabled (Fn key has no effect)"),
        ),
        1 => (
            tr!("media keys first"),
            tr!("fkeyslast (media keys by default, Fn+Fx = F-key)"),
        ),
        2 => (
            tr!("F1–F12 first"),
            tr!("fkeysfirst (F-keys by default, Fn+Fx = media key)"),
        ),
        // The kernel's auto is fkeyslast on Apple keyboards.
        3 => (
            tr!("media keys first (auto)"),
            tr!("auto (media keys by default on Apple keyboards, Fn+Fx = F-key)"),
        ),
        4 => (
            tr!("F-keys disabled"),
            tr!("fkeysdisabled (F-keys disabled, media keys only)"),
        ),
        _ => return None,
    })
}

/// Short wording of an Fn mode (`None` = a value that means nothing).
#[must_use]
pub fn fn_mode_label(mode: i32) -> Option<String> {
    fn_mode_words(mode).map(|(short, _)| short)
}

/// Long wording of an Fn mode, with the kernel name (`None` = a value that means nothing).
#[must_use]
pub fn fn_mode_description(mode: i32) -> Option<String> {
    fn_mode_words(mode).map(|(_, long)| long)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist() {
        assert!(Param::FnMode.valid(4) && !Param::FnMode.valid(5) && !Param::FnMode.valid(-1));
        assert!(Param::SwapOptCmd.valid(0) && !Param::SwapOptCmd.valid(3));
        assert!(Param::IsoLayout.valid(-1) && !Param::IsoLayout.valid(2));
        assert_eq!(
            Param::ALL.map(Param::name),
            ["fnmode", "swap_opt_cmd", "iso_layout"]
        );
    }

    #[test]
    fn reads_from_a_fixture_dir() {
        let d = std::env::temp_dir().join(format!("akm-hidp-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("fnmode"), "2\n").unwrap();
        std::fs::write(d.join("iso_layout"), "-1\n").unwrap();
        std::fs::write(d.join("swap_opt_cmd"), "9\n").unwrap();
        assert_eq!(Param::FnMode.read_in(&d), Some(2));
        assert_eq!(Param::IsoLayout.read_in(&d), Some(-1));
        assert_eq!(Param::SwapOptCmd.read_in(&d), None, "out of range");
        assert_eq!(Param::FnMode.read_in(Path::new("/nonexistent")), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fn_mode_words_cover_every_valid_value_and_agree() {
        for m in -1..=5 {
            assert_eq!(fn_mode_label(m).is_some(), Param::FnMode.valid(m), "{m}");
            assert_eq!(
                fn_mode_description(m).is_some(),
                Param::FnMode.valid(m),
                "{m}"
            );
        }
        // auto behaves as fkeyslast on Apple keyboards: both wordings say media keys first.
        assert!(fn_mode_label(3).unwrap().starts_with("media keys first"));
        assert!(fn_mode_description(3)
            .unwrap()
            .contains("media keys by default"));
        assert_eq!(fn_mode_label(4).as_deref(), Some("F-keys disabled"));
    }
}
