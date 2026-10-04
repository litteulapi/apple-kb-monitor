//! What the clients share on top of [`crate::keymap`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::keymap::{
    self, display_code, effective_table, fn_table, overrides_for, row_note, xkb_of, HidState,
    HwdbRecord, Keymap, KeymapError, PhysKey, ALU_WIRELESS_PIDS, PHYS_KEYS,
};
use crate::tr;

/// Model shown when no Apple aluminium keyboard is connected.
pub const DEFAULT_PID: u16 = 0x0256;

/// PID of the first connected Apple aluminium wireless keyboard, read from the `modalias` of the
/// input devices (sysfs attributes only: no event node, no hidraw is opened).
#[must_use]
pub fn detect_pid_in(class_input: &Path) -> Option<u16> {
    let mut found = Vec::new();
    for e in std::fs::read_dir(class_input).ok()?.flatten() {
        if !e.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let Ok(m) = std::fs::read_to_string(e.path().join("device/modalias")) else {
            continue;
        };
        if let Some(pid) = pid_from_modalias(&m) {
            found.push(pid);
        }
    }
    found.sort_unstable();
    found.into_iter().next()
}

#[must_use]
pub fn detect_pid() -> Option<u16> {
    detect_pid_in(Path::new("/sys/class/input"))
}

/// `input:b0005v05ACp0256e0050-e0,...` → 0x0256 (ALU wireless only).
#[must_use]
pub fn pid_from_modalias(m: &str) -> Option<u16> {
    let rest = m.trim().strip_prefix("input:b0005v05ACp")?;
    let pid = u16::from_str_radix(rest.get(..4)?, 16).ok()?;
    ALU_WIRELESS_PIDS.contains(&pid).then_some(pid)
}

fn side(code: u16) -> Value {
    let x = xkb_of(code);
    json!({
        "code": display_code(code),
        "keysym": x.map(|x| x.keysym),
        "qt_key": x.map(|x| x.qt_key),
        "qt_name": x.map(|x| x.qt_name),
    })
}

/// Keys shown: the top row (F1..F12, Eject) or all known keys.
#[must_use]
pub fn keys(all: bool) -> Vec<&'static PhysKey> {
    PHYS_KEYS.iter().filter(|k| all || k.top_row).collect()
}

/// Effective table (schema 1).
#[must_use]
pub fn table_json(
    pid: Option<u16>,
    p: &HidState,
    installed: &Result<Vec<HwdbRecord>, KeymapError>,
    km: &Keymap,
    all: bool,
) -> Value {
    let shown_pid = pid.unwrap_or(DEFAULT_PID);
    let empty = Vec::new();
    let inst = installed.as_ref().unwrap_or(&empty);
    let ov = overrides_for(inst, shown_pid);
    let rows: Vec<Value> = effective_table(shown_pid, p, &ov, &keys(all))
        .iter()
        .map(|r| {
            json!({
                "key": r.key.id,
                "legend": r.key.legend,
                "scancode": format!("0x{:x}", r.key.scancode),
                "base": display_code(r.base),
                "remapped": r.remapped,
                "plain": side(r.plain),
                "fn": side(r.with_fn),
                "note": row_note(r, p).map(crate::i18n::gettext),
            })
        })
        .collect();
    let prof = km.active_profile();
    let params: BTreeMap<&str, Option<i32>> = keymap::KERNEL_PARAMS
        .iter()
        .map(|k| (k.name, p.get(k.name)))
        .collect();
    json!({
        "schema": 1,
        "pid": format!("{shown_pid:04x}"),
        "connected": pid.is_some(),
        "fn_table": fn_table(shown_pid).map(|_| "magic_keyboard_alu_fn_keys"),
        "module_loaded": p.loaded(),
        "params": params,
        "installed": match installed {
            Ok(r) => json!({"path": keymap::HWDB_PATH, "present": !r.is_empty(), "overrides": ov.len()}),
            Err(e) => json!({"path": keymap::HWDB_PATH, "error": e.to_string()}),
        },
        "profile": km.active,
        "preset": prof.preset.map(super::keymap::Preset::name),
        "pending": !plan(km, p, inst).is_empty(),
        "rows": rows,
    })
}

/// Keymap as JSON (for the settings module's keymap page: profiles, presets, parameters).
#[must_use]
pub fn keymap_json(km: &Keymap, path: &Path) -> Value {
    let profiles: BTreeMap<&str, Value> = km
        .profiles
        .iter()
        .map(|(n, p)| {
            let keys: BTreeMap<String, String> = p
                .keys
                .iter()
                .map(|(sc, c)| (keymap::scancode_label(*sc), display_code(*c)))
                .collect();
            (
                n.as_str(),
                json!({
                    "preset": p.preset.map(super::keymap::Preset::name),
                    "models": p.models.iter().map(|m| format!("05ac:{m:04x}")).collect::<Vec<_>>(),
                    "params": p.params,
                    "effective_params": p.effective_params(),
                    "keys": keys,
                }),
            )
        })
        .collect();
    let presets: Vec<Value> = keymap::Preset::ALL
        .iter()
        .map(|p| json!({"name": p.name(), "title": p.title(), "params": p.params().iter().map(|(n, v)| ((*n).to_string(), *v)).collect::<BTreeMap<_, _>>()}))
        .collect();
    let params: Vec<Value> = keymap::KERNEL_PARAMS
        .iter()
        .map(|k| json!({"name": k.name, "min": k.min, "max": k.max, "help": k.help}))
        .collect();
    json!({
        "schema": 1,
        "path": path.display().to_string(),
        "active": km.active,
        "profiles": profiles,
        "presets": presets,
        "kernel_params": params,
        "keys": PHYS_KEYS.iter().map(|k| json!({"id": k.id, "legend": k.legend, "scancode": format!("0x{:x}", k.scancode)})).collect::<Vec<_>>(),
    })
}

/// What `apply` must do for the active profile.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplyPlan {
    /// hwdb content to install (`None` = keep / remove, see `remove_hwdb`).
    pub hwdb: Option<String>,
    /// The profile has no override but a file is installed: remove it.
    pub remove_hwdb: bool,
    /// Parameters whose current value differs (or is unknown).
    pub params: Vec<(String, i32)>,
}

impl ApplyPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hwdb.is_none() && !self.remove_hwdb && self.params.is_empty()
    }
}

/// Default profile on a default system = empty plan ("change nothing").
#[must_use]
pub fn plan(km: &Keymap, p: &HidState, installed: &[HwdbRecord]) -> ApplyPlan {
    let prof = km.active_profile();
    // Entries equal to the kernel default change nothing: compare without them.
    let strip = |recs: &[HwdbRecord]| -> Vec<HwdbRecord> {
        recs.iter()
            .map(|r| HwdbRecord {
                pid: r.pid,
                keys: overrides_for(std::slice::from_ref(r), r.pid),
            })
            .filter(|r| !r.keys.is_empty())
            .collect()
    };
    let want = strip(&prof.hwdb_records());
    let have = strip(installed);
    let mut out = ApplyPlan::default();
    if want.is_empty() {
        out.remove_hwdb = !have.is_empty();
    } else if want != have {
        out.hwdb = Some(keymap::render_hwdb(&km.active, &want));
    }
    for (n, v) in prof.effective_params() {
        if p.get(&n) != Some(v) {
            out.params.push((n, v));
        }
    }
    out
}

/// Fixed hand-over file of the keymap helper for the current user.
#[must_use]
pub fn source_path() -> PathBuf {
    // SAFETY: getuid(2) has no failure mode.
    let uid = unsafe { libc::getuid() };
    crate::paths::keymap_source_for_uid(uid)
}

/// Writes the hwdb to hand over (0644, owned by the user, single link).
///
/// # Errors
///
/// [`KeymapError`] wrapping the I/O error.
// Own copy, not fsutil::write_atomic: the root helper wants a plain 0644 file, never a link to follow.
pub fn write_source(path: &Path, content: &str) -> Result<(), KeymapError> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let io = |e: std::io::Error| KeymapError::new(format!("{}: {e}", path.display()));
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(io)?;
    }
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("hwdb.{}.{n}.tmp", std::process::id()));
    let staged = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&tmp, path)
    })();
    if staged.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    staged.map_err(io)
}

/// The hand-over lock, held by the daemon and akmctl alike for a whole privileged apply or reset.
#[derive(Debug)]
pub struct HandOver {
    source: PathBuf,
    _lock: std::fs::File,
}

impl HandOver {
    /// Takes the lock of the hand-over directory of `source`.
    ///
    /// # Errors
    ///
    /// [`KeymapError`] when another process (daemon or akmctl) holds it, or on an I/O error.
    pub fn lock(source: &Path) -> Result<Self, KeymapError> {
        Ok(Self {
            source: source.to_path_buf(),
            _lock: lock_source(source)?,
        })
    }

    /// Runs `run` while the hand-over file holds `hwdb`, then removes the file.
    ///
    /// # Errors
    ///
    /// Why the file could not be written, or the error of `run`.
    pub fn with_hwdb<T>(
        &self,
        hwdb: &str,
        run: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        write_source(&self.source, hwdb).map_err(|e| e.to_string())?;
        let r = run();
        let _ = std::fs::remove_file(&self.source);
        r
    }
}

fn lock_source(path: &Path) -> Result<std::fs::File, KeymapError> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let io = |e: std::io::Error| KeymapError::new(format!("{}: {e}", path.display()));
    let d = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(d).map_err(io)?;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(d.join("keymap.lock"))
        .map_err(io)?;
    // SAFETY: flock(2) on a descriptor owned by `f`, which outlives the call.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::WouldBlock {
            return Err(io(e));
        }
        return Err(KeymapError::new(tr!("another keymap apply is in progress")));
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Profile;

    fn cur() -> HidState {
        HidState {
            fnmode: Some(1),
            ..HidState::KERNEL_DEFAULT
        }
    }

    #[test]
    fn modalias() {
        assert_eq!(
            pid_from_modalias("input:b0005v05ACp0256e0050-e0,1,4,11,14,k71\n"),
            Some(0x0256)
        );
        assert_eq!(
            pid_from_modalias("input:b0003v05ACp0256e0050-e0"),
            None,
            "USB not handled"
        );
        assert_eq!(pid_from_modalias("input:b0005v05ACp0267e0050"), None);
        assert_eq!(pid_from_modalias("input:b0005v046Dp0256e0050"), None);
    }

    #[test]
    fn detect_from_a_fake_sysfs() {
        let d = std::env::temp_dir().join(format!("akm-kt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (n, m) in [
            ("event3", "input:b0003v046Dp4002e0111-e0"),
            ("event28", "input:b0005v05ACp0256e0050-e0,1"),
        ] {
            std::fs::create_dir_all(d.join(n).join("device")).unwrap();
            std::fs::write(d.join(n).join("device/modalias"), m).unwrap();
        }
        assert_eq!(detect_pid_in(&d), Some(0x0256));
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(detect_pid_in(&d), None);
    }

    #[test]
    fn default_changes_nothing() {
        let km = Keymap::default();
        assert!(plan(&km, &cur(), &[]).is_empty());
        for s in [
            HidState {
                swap_opt_cmd: Some(1),
                ..cur()
            },
            HidState {
                fnmode: Some(2),
                ..cur()
            },
            HidState::default(),
        ] {
            assert!(plan(&km, &s, &[]).is_empty(), "{s:?}");
        }
        let t = table_json(Some(0x0256), &cur(), &Ok(vec![]), &km, false);
        assert_eq!(t["pending"], false);
        assert_eq!(t["rows"][0]["plain"]["qt_name"], "Monitor Brightness Down");
        assert_eq!(t["rows"][0]["fn"]["code"], "KEY_F1");
        assert_eq!(t["rows"].as_array().unwrap().len(), 13);
        assert_eq!(
            table_json(None, &cur(), &Ok(vec![]), &km, true)["rows"]
                .as_array()
                .unwrap()
                .len(),
            PHYS_KEYS.len()
        );
    }

    #[test]
    fn plan_installs_removes_and_sets_params() {
        let mut km = Keymap::default();
        km.profile_mut("default").unwrap().keys.insert(0x7003f, 183);
        let pl = plan(&km, &cur(), &[]);
        assert!(pl
            .hwdb
            .as_deref()
            .unwrap()
            .contains(" KEYBOARD_KEY_7003f=f13\n"));
        assert!(pl.params.is_empty(), "{:?}", pl.params);
        let (_, inst) = keymap::parse_hwdb(pl.hwdb.as_deref().unwrap()).unwrap();
        assert!(plan(&km, &cur(), &inst).is_empty());
        km.profile_mut("default").unwrap().keys.insert(0x7003e, 63);
        assert!(plan(&km, &cur(), &inst).is_empty());
        km.profile_mut("default").unwrap().keys.clear();
        assert!(plan(&km, &cur(), &inst).remove_hwdb);
        km.profile_mut("default").unwrap().preset = Some(keymap::Preset::FKeys);
        assert_eq!(
            plan(&km, &cur(), &[]).params,
            vec![("fnmode".to_string(), 2)]
        );
        let p = HidState::default();
        let apple = Profile {
            preset: Some(keymap::Preset::Apple),
            ..Profile::default()
        };
        assert_eq!(
            plan(
                &Keymap {
                    active: "default".into(),
                    profiles: BTreeMap::from([("default".into(), apple)])
                },
                &p,
                &[]
            )
            .params
            .len(),
            2
        );
    }

    #[test]
    fn write_source_mode() {
        use std::os::unix::fs::MetadataExt;
        let d = std::env::temp_dir().join(format!("akm-kt-src-{}", std::process::id()));
        let p = d.join("apple-kb-monitor/keymap.hwdb");
        write_source(&p, "x\n").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().mode() & 0o777, 0o644);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn lock_source_is_exclusive() {
        let d = std::env::temp_dir().join(format!("akm-kt-lock-{}", std::process::id()));
        let p = d.join("apple-kb-monitor/keymap.hwdb");
        let held = lock_source(&p).unwrap();
        assert!(
            lock_source(&p).is_err(),
            "a second run must not touch the hand-over file"
        );
        drop(held);
        assert!(lock_source(&p).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn hand_over_file_lives_only_during_the_run() {
        let d = std::env::temp_dir().join(format!("akm-kt-hand-{}", std::process::id()));
        let p = d.join("apple-kb-monitor/keymap.hwdb");
        let hand = HandOver::lock(&p).unwrap();
        assert!(HandOver::lock(&p).is_err());
        let seen = hand.with_hwdb("hwdb\n", || {
            std::fs::read_to_string(&p).map_err(|e| e.to_string())
        });
        assert_eq!(seen.unwrap(), "hwdb\n");
        assert!(!p.exists());
        drop(hand);
        assert!(HandOver::lock(&p).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
