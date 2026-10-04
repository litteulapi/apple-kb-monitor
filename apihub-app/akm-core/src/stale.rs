//! What still runs the old code after an upgrade.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Binaries of the package that keep running across an upgrade, with the exact command that
/// restarts them.
pub const WATCHED: [(&str, &str, &str); 2] = [
    (
        "apple-kb-monitord",
        "/usr/bin/apple-kb-monitord",
        "systemctl --user restart apple-kb-monitord.service",
    ),
    (
        "apihub-app",
        "/usr/bin/apihub-app",
        "pkill -x apihub-app && apihub-app &",
    ),
];
/// Installed with the widget: its time is the widget's installation time.
pub const WIDGET_METADATA: &str =
    "/usr/share/plasma/plasmoids/com.agenceapi.devicehub/metadata.json";
/// Restarts the Plasma shell of the session (Plasma 6, systemd startup).
pub const PLASMA_RESTART: &str = "systemctl --user restart plasma-plasmashell.service";
const USER_HZ: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// Process `pid` runs `name` from a file that the upgrade replaced.
    DeletedExe {
        pid: u32,
        name: &'static str,
        path: &'static str,
        fix: &'static str,
    },
    /// plasmashell `pid` started (Unix s) before the widget was installed.
    PlasmaOlder {
        pid: u32,
        started: u64,
        installed: u64,
    },
}

impl Stale {
    /// Stable identifier, for the readers that translate.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::DeletedExe { .. } => "versions.deleted-exe",
            Self::PlasmaOlder { .. } => "versions.plasma-older",
        }
    }
    /// Values of the message, in the order of [`Stale::text`].
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::DeletedExe {
                pid, name, path, ..
            } => {
                vec![(*name).to_string(), pid.to_string(), (*path).to_string()]
            }
            Self::PlasmaOlder { pid, .. } => vec![pid.to_string()],
        }
    }
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::DeletedExe { pid, name, path, .. } => format!(
                "{name} (pid {pid}) still runs the replaced binary {path} (deleted): the old version"
            ),
            Self::PlasmaOlder { pid, .. } => format!(
                "plasmashell (pid {pid}) started before the widget was installed: it shows the old widget"
            ),
        }
    }
    /// The exact command to run.
    #[must_use]
    pub fn fix(&self) -> &'static str {
        match self {
            Self::DeletedExe { fix, .. } => fix,
            Self::PlasmaOlder { .. } => PLASMA_RESTART,
        }
    }
}

/// The watched binary that an `exe` link names as deleted, if any.
#[must_use]
pub fn deleted_exe(link: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let path = link.strip_suffix(" (deleted)")?;
    WATCHED.iter().find(|(_, p, _)| *p == path).copied()
}

/// Start time from the text of `/proc/<pid>/stat` and the boot time.
#[must_use]
pub fn start_time(stat: &str, btime: u64) -> Option<u64> {
    // The command name (field 2) may hold spaces and parentheses.
    let rest = &stat[stat.rfind(')')? + 1..];
    let ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    Some(btime + ticks / USER_HZ)
}

/// `btime` of the text of `/proc/stat`.
#[must_use]
pub fn boot_time(stat: &str) -> Option<u64> {
    stat.lines()
        .find_map(|l| l.strip_prefix("btime "))
        .and_then(|v| v.trim().parse().ok())
}

/// True when two versions name the same build.
#[must_use]
pub fn same_version(a: &str, b: &str) -> bool {
    let split = |v: &str| match v.trim().rsplit_once('-') {
        Some((u, r)) if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit() || c == '.') => {
            (u.to_string(), Some(r.to_string()))
        }
        _ => (v.trim().to_string(), None),
    };
    let ((ua, ra), (ub, rb)) = (split(a), split(b));
    ua == ub && (ra.is_none() || rb.is_none() || ra == rb)
}

/// One process seen in `/proc`, as [`classify`] needs it.
#[derive(Debug, Clone, Default)]
pub struct Proc {
    pub pid: u32,
    pub comm: String,
    /// Target of `/proc/<pid>/exe` (readable for the user's own processes).
    pub exe: Option<String>,
    pub started: Option<u64>,
}

/// Pure: what is stale among the user's processes, given the widget's installation time.
pub fn classify(procs: &[Proc], widget_installed: Option<u64>) -> Vec<Stale> {
    let mut out = Vec::new();
    for p in procs {
        if let Some((name, path, fix)) = p.exe.as_deref().and_then(deleted_exe) {
            out.push(Stale::DeletedExe {
                pid: p.pid,
                name,
                path,
                fix,
            });
        }
        if p.comm == "plasmashell" {
            if let (Some(started), Some(installed)) = (p.started, widget_installed) {
                if started < installed {
                    out.push(Stale::PlasmaOlder {
                        pid: p.pid,
                        started,
                        installed,
                    });
                }
            }
        }
    }
    out
}

/// The current user's processes (read-only scan of `/proc`).
#[must_use]
pub fn user_procs() -> Vec<Proc> {
    // SAFETY: getuid() has no precondition and cannot fail.
    let uid = unsafe { libc::getuid() };
    let btime = std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| boot_time(&s));
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let base = e.path();
        if std::fs::metadata(&base).map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let comm = std::fs::read_to_string(base.join("comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let exe = std::fs::read_link(base.join("exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let started = btime.and_then(|b| {
            std::fs::read_to_string(base.join("stat"))
                .ok()
                .and_then(|s| start_time(&s, b))
        });
        out.push(Proc {
            pid,
            comm,
            exe,
            started,
        });
    }
    out
}

/// Installation time of the widget (latest of mtime and ctime), if installed.
#[must_use]
pub fn widget_installed(path: &Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    Some(m.mtime().max(m.ctime()).max(0).cast_unsigned())
}

/// Everything stale for the current user, now.
#[must_use]
pub fn scan() -> Vec<Stale> {
    classify(&user_procs(), widget_installed(Path::new(WIDGET_METADATA)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_binaries_of_the_package_only() {
        assert_eq!(
            deleted_exe("/usr/bin/apihub-app (deleted)").map(|w| w.0),
            Some("apihub-app")
        );
        assert_eq!(
            deleted_exe("/usr/bin/apple-kb-monitord (deleted)").map(|w| w.2),
            Some("systemctl --user restart apple-kb-monitord.service")
        );
        assert_eq!(deleted_exe("/usr/bin/apihub-app"), None);
        assert_eq!(deleted_exe("/usr/bin/firefox (deleted)"), None);
        assert_eq!(deleted_exe("/home/u/apihub-app (deleted)"), None);
    }

    #[test]
    fn start_time_reads_field_22() {
        let stat = "1443 (my (odd) name) S 1 1443 1443 0 -1 4194560 1 2 3 4 5 6 7 8 20 0 1 0 12345 1000 10";
        assert_eq!(start_time(stat, 1_700_000_000), Some(1_700_000_000 + 123));
        assert_eq!(start_time("garbage", 1), None);
        assert_eq!(
            boot_time("cpu 1 2\nbtime 1759470000\nprocesses 5\n"),
            Some(1_759_470_000)
        );
    }

    #[test]
    fn versions_compare_the_release() {
        assert!(same_version("3.1.0-27", "3.1.0-27"));
        assert!(!same_version("3.1.0-26", "3.1.0-27"));
        assert!(same_version("3.1.0", "3.1.0-27")); // a build without the release
        assert!(!same_version("3.0.9", "3.1.0-27"));
        assert!(!same_version("3.0.9-1", "3.1.0"));
    }

    #[test]
    fn classify_finds_deleted_exe_and_old_plasmashell() {
        let procs = [
            Proc {
                pid: 10,
                comm: "apihub-app".into(),
                exe: Some("/usr/bin/apihub-app (deleted)".into()),
                started: Some(5),
            },
            Proc {
                pid: 11,
                comm: "apple-kb-monito".into(),
                exe: Some("/usr/bin/apple-kb-monitord".into()),
                started: Some(5),
            },
            Proc {
                pid: 12,
                comm: "plasmashell".into(),
                exe: Some("/usr/bin/plasmashell".into()),
                started: Some(100),
            },
        ];
        let s = classify(&procs, Some(200));
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[0].id(), "versions.deleted-exe");
        assert_eq!(s[0].args(), ["apihub-app", "10", "/usr/bin/apihub-app"]);
        assert_eq!(s[0].fix(), "pkill -x apihub-app && apihub-app &");
        assert_eq!(
            s[1],
            Stale::PlasmaOlder {
                pid: 12,
                started: 100,
                installed: 200
            }
        );
        assert_eq!(s[1].fix(), PLASMA_RESTART);
        assert!(
            classify(&procs[2..], Some(50)).is_empty(),
            "{:?}",
            classify(&procs[2..], Some(50))
        );
        assert!(
            classify(&procs[2..], None).is_empty(),
            "{:?}",
            classify(&procs[2..], None)
        );
    }

    #[test]
    fn the_package_build_injects_the_full_version() {
        let pkgbuild = include_str!("../../../PKGBUILD");
        assert!(
            pkgbuild.contains("export AKM_PKG_VERSION=\"$pkgver-$pkgrel\""),
            "PKGBUILD build() must export AKM_PKG_VERSION"
        );
    }
}
