//! Fn-key mode of the `hid_apple` kernel module (global to every Apple keyboard).

use akm_core::hid_params::Param;
use akm_core::tr;
use std::path::Path;

pub const SYSFS_FNMODE: &str = "/sys/module/hid_apple/parameters/fnmode";

pub fn label(mode: u8) -> String {
    akm_core::hid_params::fn_mode_description(i32::from(mode)).unwrap_or_else(|| tr!("unknown"))
}

pub fn parse_mode(s: &str) -> Result<u8, String> {
    match s.as_bytes() {
        [d @ b'0'..=b'9'] if Param::FnMode.valid(i32::from(d - b'0')) => Ok(d - b'0'),
        _ => Err(tr!(
            "invalid Fn mode {mode}: expected 0, 1, 2, 3 or 4",
            mode = format!("{s:?}")
        )),
    }
}

pub fn parse_sysfs(content: &str) -> Option<u8> {
    parse_mode(content.trim()).ok()
}

pub fn read_from(path: &Path) -> Result<u8, String> {
    let s = std::fs::read_to_string(path).map_err(|e| {
        tr!(
            "cannot read {path} (hid_apple loaded?): {e}",
            path = path.display(),
            e = e
        )
    })?;
    parse_sysfs(&s).ok_or_else(|| {
        tr!(
            "unexpected content in {path}: {content}",
            path = path.display(),
            content = format!("{:?}", s.trim())
        )
    })
}

pub fn read() -> Result<u8, String> {
    read_from(Path::new(SYSFS_FNMODE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_bounds() {
        for n in 0..=4 {
            assert_eq!(parse_mode(&n.to_string()), Ok(u8::try_from(n).unwrap()));
        }
        for bad in ["5", "-1", "", "01", "a", "1 ", "+2"] {
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
        let k = akm_core::keymap::kernel_param("fnmode").unwrap();
        for n in k.min..=k.max {
            let n = u8::try_from(n).unwrap();
            assert_ne!(label(n), "unknown");
            assert_eq!(
                parse_mode(&n.to_string()),
                Ok(n),
                "same range as KERNEL_PARAMS"
            );
            assert_eq!(parse_sysfs(&format!("{n}\n")), Some(n));
        }
    }
}
