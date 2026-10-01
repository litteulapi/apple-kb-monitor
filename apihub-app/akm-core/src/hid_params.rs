//! `hid_apple` module parameters (`/sys/module/hid_apple/parameters`):
//! reading and the whitelist of values the privileged helper accepts
//! (F07–F09, write path in the daemon). Reading needs no privilege.

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
    pub fn name(self) -> &'static str {
        match self {
            Param::FnMode => "fnmode",
            Param::SwapOptCmd => "swap_opt_cmd",
            Param::IsoLayout => "iso_layout",
        }
    }

    pub fn valid(self, v: i32) -> bool {
        match self {
            Param::FnMode => (0..=3).contains(&v),
            Param::SwapOptCmd => (0..=2).contains(&v),
            Param::IsoLayout => (-1..=1).contains(&v),
        }
    }

    /// Current value under `dir` (`None` if absent or unreadable).
    pub fn read_in(self, dir: &Path) -> Option<i32> {
        std::fs::read_to_string(dir.join(self.name()))
            .ok()?
            .trim()
            .parse()
            .ok()
            .filter(|v| self.valid(*v))
    }

    /// Current value, [`UNKNOWN`] if it cannot be read.
    pub fn read(self) -> i32 {
        self.read_in(Path::new(SYSFS_DIR)).unwrap_or(UNKNOWN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist() {
        assert!(Param::FnMode.valid(2) && !Param::FnMode.valid(4) && !Param::FnMode.valid(-1));
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
}
