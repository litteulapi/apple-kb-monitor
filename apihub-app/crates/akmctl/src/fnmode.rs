//! Fn-key mode of the `hid_apple` kernel module (global to every Apple keyboard).

use std::path::Path;

pub const SYSFS_FNMODE: &str = "/sys/module/hid_apple/parameters/fnmode";

/// Human label of a mode (kernel documentation of `hid_apple.fnmode`).
pub fn label(mode: u8) -> &'static str {
    match mode {
        0 => "disabled (Fn key has no effect)",
        1 => "fkeyslast (media keys by default, Fn+Fx = F-key)",
        2 => "fkeysfirst (F-keys by default, Fn+Fx = media key)",
        3 => "auto (F-keys by default, media keys on Apple-layout apps)",
        _ => "unknown",
    }
}

/// Parse the value given on the command line (`clap` value parser): 0..=3.
pub fn parse_mode(s: &str) -> Result<u8, String> {
    match s.as_bytes() {
        [d @ b'0'..=b'3'] => Ok(d - b'0'),
        _ => Err(format!("invalid Fn mode {s:?}: expected 0, 1, 2 or 3")),
    }
}

/// Parse the content of the sysfs file (`"2\n"`).
pub fn parse_sysfs(content: &str) -> Option<u8> {
    parse_mode(content.trim()).ok()
}

/// Read the current mode. `Err` if `hid_apple` is not loaded.
pub fn read_from(path: &Path) -> Result<u8, String> {
    let s = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {} (hid_apple loaded?): {e}", path.display()))?;
    parse_sysfs(&s).ok_or_else(|| format!("unexpected content in {}: {:?}", path.display(), s.trim()))
}

pub fn read() -> Result<u8, String> {
    read_from(Path::new(SYSFS_FNMODE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_bounds() {
        for n in 0..=3 {
            assert_eq!(parse_mode(&n.to_string()), Ok(n as u8));
        }
        for bad in ["4", "-1", "", "01", "a", "1 ", "+2"] {
            assert!(parse_mode(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn parse_sysfs_trims() {
        assert_eq!(parse_sysfs("2\n"), Some(2));
        assert_eq!(parse_sysfs("garbage"), None);
        assert_eq!(parse_sysfs(""), None);
    }

    #[test]
    fn read_missing_file_is_error() {
        assert!(read_from(Path::new("/nonexistent/akm/fnmode")).is_err());
    }

    #[test]
    fn labels_cover_all_modes() {
        for n in 0..=3 {
            assert_ne!(label(n), "unknown");
        }
    }
}
