//! What the clients (akmctl, daemon, window) share on top of [`crate::keymap`]
//! (#247): the effective table as JSON, the keyboard detection from sysfs,
//! and the apply plan (what to hand to the two privileged helpers).
//! No D-Bus here: the KDE action of a Qt key is resolved by each client
//! (`org.kde.kglobalaccel` `action(i)`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::keymap::{
    self, display_code, effective_table, fn_table, overrides_for, row_note, xkb_of, HidState, HwdbRecord, Keymap,
    KeymapError, PhysKey, ALU_WIRELESS_PIDS, PHYS_KEYS,
};

/// Model shown when no Apple aluminium keyboard is connected.
pub const DEFAULT_PID: u16 = 0x0256;

/// PID of the first connected Apple aluminium wireless keyboard, read from
/// the `modalias` of the input devices (sysfs attributes only: no event
/// node, no hidraw is opened).
pub fn detect_pid_in(class_input: &Path) -> Option<u16> {
    let mut found = Vec::new();
    for e in std::fs::read_dir(class_input).ok()?.flatten() {
        if !e.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let Ok(m) = std::fs::read_to_string(e.path().join("device/modalias")) else { continue };
        if let Some(pid) = pid_from_modalias(&m) {
            found.push(pid);
        }
    }
    found.sort_unstable();
    found.into_iter().next()
}

pub fn detect_pid() -> Option<u16> {
    detect_pid_in(Path::new("/sys/class/input"))
}

/// `input:b0005v05ACp0256e0050-e0,...` → 0x0256 (ALU wireless only).
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
pub fn keys(all: bool) -> Vec<&'static PhysKey> {
    PHYS_KEYS.iter().filter(|k| all || k.top_row).collect()
}

/// Effective table (schema 1). `installed` = records of the installed hwdb
/// file, `km` = the user's keymap (for the "pending" flag).
pub fn table_json(pid: Option<u16>, p: &HidState, installed: &Result<Vec<HwdbRecord>, KeymapError>, km: &Keymap, all: bool) -> Value {
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
                "note": row_note(r, p),
            })
        })
        .collect();
    let prof = km.active_profile();
    let params: BTreeMap<&str, Option<i32>> = keymap::KERNEL_PARAMS.iter().map(|k| (k.name, p.get(k.name))).collect();
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
        "preset": prof.preset.map(|p| p.name()),
        "pending": plan(km, p, inst).map(|pl| !pl.is_empty()).unwrap_or(false),
        "rows": rows,
    })
}

/// Keymap as JSON (for the window: profiles, presets, parameters).
pub fn keymap_json(km: &Keymap, path: &Path) -> Value {
    let profiles: BTreeMap<&str, Value> = km
        .profiles
        .iter()
        .map(|(n, p)| {
            let keys: BTreeMap<String, String> =
                p.keys.iter().map(|(sc, c)| (keymap::scancode_label(*sc), display_code(*c))).collect();
            (
                n.as_str(),
                json!({
                    "preset": p.preset.map(|p| p.name()),
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
        .map(|p| json!({"name": p.name(), "title": p.title(), "params": p.params().iter().map(|(n, v)| (n.to_string(), *v)).collect::<BTreeMap<_, _>>()}))
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
    pub fn is_empty(&self) -> bool {
        self.hwdb.is_none() && !self.remove_hwdb && self.params.is_empty()
    }
}

/// Default profile on a default system = empty plan ("ne rien changer").
pub fn plan(km: &Keymap, p: &HidState, installed: &[HwdbRecord]) -> Result<ApplyPlan, KeymapError> {
    let prof = km.active_profile();
    // Entries equal to the kernel default change nothing: compare without them.
    let strip = |recs: &[HwdbRecord]| -> Vec<HwdbRecord> {
        recs.iter()
            .map(|r| HwdbRecord { pid: r.pid, keys: overrides_for(std::slice::from_ref(r), r.pid) })
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
    Ok(out)
}

/// Fixed hand-over file of the keymap helper for the current user.
pub fn source_path() -> PathBuf {
    // SAFETY: getuid(2) has no failure mode.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/apple-kb-monitor/keymap.hwdb"))
}

/// Writes the hwdb to hand over (0644, owned by the user, single link).
pub fn write_source(path: &Path, content: &str) -> Result<(), KeymapError> {
    use std::os::unix::fs::PermissionsExt;
    let io = |e: std::io::Error| KeymapError::new(format!("{}: {e}", path.display()));
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(io)?;
    }
    let tmp = path.with_extension("hwdb.tmp");
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(&tmp, content).map_err(io)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Profile;

    fn cur() -> HidState {
        HidState { fnmode: Some(1), ..HidState::KERNEL_DEFAULT }
    }

    #[test]
    fn modalias() {
        assert_eq!(pid_from_modalias("input:b0005v05ACp0256e0050-e0,1,4,11,14,k71\n"), Some(0x0256));
        assert_eq!(pid_from_modalias("input:b0003v05ACp0256e0050-e0"), None, "USB not handled");
        assert_eq!(pid_from_modalias("input:b0005v05ACp0267e0050"), None);
        assert_eq!(pid_from_modalias("input:b0005v046Dp0256e0050"), None);
    }

    #[test]
    fn detect_from_a_fake_sysfs() {
        let d = std::env::temp_dir().join(format!("akm-kt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (n, m) in [("event3", "input:b0003v046Dp4002e0111-e0"), ("event28", "input:b0005v05ACp0256e0050-e0,1")] {
            std::fs::create_dir_all(d.join(n).join("device")).unwrap();
            std::fs::write(d.join(n).join("device/modalias"), m).unwrap();
        }
        assert_eq!(detect_pid_in(&d), Some(0x0256));
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(detect_pid_in(&d), None);
    }

    /// Default keymap (no file): nothing to do whatever the current
    /// parameters (measured 2026-10-01: swap_opt_cmd went 0 → 1 by hand).
    #[test]
    fn default_changes_nothing() {
        let km = Keymap::default();
        assert!(plan(&km, &cur(), &[]).unwrap().is_empty());
        for s in [HidState { swap_opt_cmd: Some(1), ..cur() }, HidState { fnmode: Some(2), ..cur() }, HidState::default()] {
            assert!(plan(&km, &s, &[]).unwrap().is_empty(), "{s:?}");
        }
        let t = table_json(Some(0x0256), &cur(), &Ok(vec![]), &km, false);
        assert_eq!(t["pending"], false);
        assert_eq!(t["rows"][0]["plain"]["qt_name"], "Monitor Brightness Down");
        assert_eq!(t["rows"][0]["fn"]["code"], "KEY_F1");
        assert_eq!(t["rows"].as_array().unwrap().len(), 13);
        assert_eq!(table_json(None, &cur(), &Ok(vec![]), &km, true)["rows"].as_array().unwrap().len(), PHYS_KEYS.len());
    }

    #[test]
    fn plan_installs_removes_and_sets_params() {
        let mut km = Keymap::default();
        km.profile_mut("default").unwrap().keys.insert(0x7003f, 183);
        let pl = plan(&km, &cur(), &[]).unwrap();
        assert!(pl.hwdb.as_deref().unwrap().contains(" KEYBOARD_KEY_7003f=f13\n"));
        assert!(pl.params.is_empty());
        // already installed: nothing
        let (_, inst) = keymap::parse_hwdb(pl.hwdb.as_deref().unwrap()).unwrap();
        assert!(plan(&km, &cur(), &inst).unwrap().is_empty());
        // a key mapped to its own default changes nothing
        km.profile_mut("default").unwrap().keys.insert(0x7003e, 63);
        assert!(plan(&km, &cur(), &inst).unwrap().is_empty());
        // keys cleared while installed: remove
        km.profile_mut("default").unwrap().keys.clear();
        assert!(plan(&km, &cur(), &inst).unwrap().remove_hwdb);
        // preset fkeys: fnmode 2 only
        km.profile_mut("default").unwrap().preset = Some(keymap::Preset::FKeys);
        assert_eq!(plan(&km, &cur(), &[]).unwrap().params, vec![("fnmode".to_string(), 2)]);
        // unknown current value (module not loaded): set it
        let p = HidState::default();
        let apple = Profile { preset: Some(keymap::Preset::Apple), ..Profile::default() };
        assert_eq!(plan(&Keymap { active: "default".into(), profiles: BTreeMap::from([("default".into(), apple)]) }, &p, &[]).unwrap().params.len(), 2);
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
}
