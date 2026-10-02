//! Diagnostic of `hid_apple` `fnmode`: the value really applied (sysfs) and the
//! one configured for the next boot (modprobe.d), never a hard-coded text (#161).

use std::path::Path;

use akm_core::hid_params::Param;

use crate::i18n::{tr, trf};

pub const MODPROBE_DIR: &str = "/etc/modprobe.d";

/// `fnmode` set by `options hid_apple ... fnmode=N ...` lines of one file
/// (last occurrence wins, as modprobe does; comments ignored).
pub fn parse_modprobe(content: &str) -> Option<i32> {
    let mut found = None;
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        if words.next() != Some("options") || words.next() != Some("hid_apple") {
            continue;
        }
        for w in words {
            if let Some(v) = w.strip_prefix("fnmode=") {
                if let Some(n) = v.parse::<i32>().ok().filter(|n| Param::FnMode.valid(*n)) {
                    found = Some(n);
                }
            }
        }
    }
    found
}

/// Configured value over every `*.conf` of `dir`, in modprobe order (sorted).
pub fn configured_in(dir: &Path) -> Option<i32> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "conf"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|c| parse_modprobe(&c))
        .next_back()
}

/// `(ok, detail)` of the Diag row from the live value and the configured one.
pub fn verdict(live: Option<i32>, configured: Option<i32>) -> (bool, String) {
    let show = |v: Option<i32>, none: &str| v.map_or(none.to_string(), |n| n.to_string());
    let state = trf(
        "applied (sysfs) = {} · configured (modprobe.d) = {}",
        &[
            &show(live, tr("unreadable (hid_apple not loaded?)")),
            &show(configured, tr("none")),
        ],
    );
    match (live, configured) {
        (Some(l), Some(c)) if l == c => (true, state),
        (Some(_), Some(_)) => (
            false,
            trf("{} — differ: reload hid_apple or reboot", &[&state]),
        ),
        (Some(_), None) => (
            true,
            trf(
                "{} (not persistent: akmctl set fnmode N --persist)",
                &[&state],
            ),
        ),
        (None, _) => (false, state),
    }
}

/// Live read of both sources.
pub fn diagnose() -> (bool, String) {
    verdict(
        Param::FnMode.read_in(Path::new(akm_core::hid_params::SYSFS_DIR)),
        configured_in(Path::new(MODPROBE_DIR)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_options_line() {
        assert_eq!(parse_modprobe("options hid_apple fnmode=2\n"), Some(2));
        assert_eq!(
            parse_modprobe("options hid_apple iso_layout=0 fnmode=3 # x"),
            Some(3)
        );
        assert_eq!(parse_modprobe("# options hid_apple fnmode=1"), None);
        assert_eq!(parse_modprobe("options other fnmode=1"), None);
        assert_eq!(parse_modprobe("options hid_apple fnmode=9"), None);
        assert_eq!(
            parse_modprobe("options hid_apple fnmode=1\noptions hid_apple fnmode=2"),
            Some(2)
        );
    }

    #[test]
    fn verdict_flags_mismatch_and_reports_values() {
        let (ok, d) = verdict(Some(2), Some(2));
        assert!(ok && d.contains("sysfs) = 2") && d.contains("modprobe.d) = 2"));
        let (ok, d) = verdict(Some(1), Some(2));
        assert!(!ok && d.contains("differ"));
        let (ok, d) = verdict(Some(2), None);
        assert!(ok && d.contains("not persistent"));
        assert!(!verdict(None, Some(1)).0);
        assert!(!d.contains("fnmode=1 configured"));
    }

    #[test]
    fn reads_modprobe_dir_in_order() {
        let d = std::env::temp_dir().join(format!("akm-diag-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.conf"), "options hid_apple fnmode=1\n").unwrap();
        std::fs::write(d.join("z.conf"), "options hid_apple fnmode=2\n").unwrap();
        std::fs::write(d.join("zz.txt"), "options hid_apple fnmode=3\n").unwrap();
        assert_eq!(configured_in(&d), Some(2));
        let _ = std::fs::remove_dir_all(&d);
    }
}
