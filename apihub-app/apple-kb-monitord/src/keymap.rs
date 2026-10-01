//! D-Bus interface `com.agenceapi.AppleKbMonitor1.Keymap` on the root object
//! (#247): effective table of the special keys and manual key mapping.
//!
//! Methods (all JSON answers are schema 1, see `akm_core::keytable`):
//! * `KeyTable(b all) -> s`: effective table (physical key → evdev code →
//!   keysym / Qt key); the KDE action of a Qt key is resolved by the client
//!   (`org.kde.kglobalaccel` `action(i)`), never by the daemon.
//! * `Keymap() -> s`: `keymap.toml` (profiles, presets, kernel parameters).
//! * `SetKey(s profile, s key, s code) -> s`: `code` = `KEY_*`, `""` = unset;
//!   `profile` `""` = the active one. Only edits `keymap.toml`.
//! * `SetPreset(s profile, s preset) -> s`, `UseProfile(s profile) -> s`.
//! * `Apply() -> s`: installs the active profile (hwdb through
//!   `pkexec akm-keymap-helper install|remove`, parameters through
//!   `pkexec akm-helper set-params ... --persist`); one authentication at a
//!   time, cool-down as for `SetFnMode`.
//! * `Reset() -> s`: `akm-keymap-helper remove` (kernel mapping back).
//!
//! No grab, no uinput: the daemon never touches the input devices.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akm_core::keymap::{self, HidState, Keymap, Preset};
use akm_core::keytable;
use zbus::interface;

use crate::devices::unblock;
use crate::settings::{COOLDOWN, PKEXEC_PATH};

pub const KEYMAP_HELPER_PATH: &str = "/usr/lib/apple-kb-monitor/akm-keymap-helper";
pub const PARAM_HELPER_PATH: &str = crate::settings::HELPER_PATH;
const HELPER_TIMEOUT: Duration = Duration::from_secs(120);

/// Privileged side, replaceable in tests.
pub trait Privileged: Send + Sync {
    /// `akm-keymap-helper install|remove` or `akm-helper set-params ...`.
    fn run(&self, program: &str, args: &[String]) -> Result<(), String>;
}

#[derive(Debug, Default)]
struct Guard {
    running: bool,
    last_end: Option<Instant>,
}

/// pkexec with constant programs, single flight + cool-down.
#[derive(Debug, Default)]
pub struct Pkexec {
    guard: Mutex<Guard>,
}

impl Pkexec {
    fn acquire(&self) -> Result<(), String> {
        let mut g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        if g.running {
            return Err("an authentication is already pending".into());
        }
        if g.last_end.is_some_and(|t| t.elapsed() < COOLDOWN) {
            return Err("too many requests, retry shortly".into());
        }
        g.running = true;
        Ok(())
    }

    fn release(&self) {
        let mut g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        g.running = false;
        g.last_end = Some(Instant::now());
    }

    fn spawn(program: &str, args: &[String]) -> Result<(), String> {
        if program != KEYMAP_HELPER_PATH && program != PARAM_HELPER_PATH {
            return Err(format!("program not allowed: {program}"));
        }
        if !std::path::Path::new(program).is_file() {
            return Err(format!("privileged helper not installed ({program})"));
        }
        let mut child = Command::new(PKEXEC_PATH)
            .arg(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run pkexec: {e}"))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => return Ok(()),
                Ok(Some(st)) => {
                    let mut msg = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        let _ = std::io::Read::read_to_string(&mut e, &mut msg);
                    }
                    return Err(format!("helper failed ({st}): {}", msg.trim()));
                }
                Ok(None) if start.elapsed() > HELPER_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("helper timed out".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

impl Privileged for Pkexec {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        self.acquire()?;
        let r = Self::spawn(program, args);
        self.release();
        r
    }
}

/// Where the interface reads and writes (constants in production).
#[derive(Debug, Clone)]
pub struct KeymapPaths {
    pub keymap: PathBuf,
    pub installed: PathBuf,
    pub source: PathBuf,
    pub sysfs: PathBuf,
}

impl Default for KeymapPaths {
    fn default() -> Self {
        Self {
            keymap: keymap::default_path(),
            installed: PathBuf::from(keymap::HWDB_PATH),
            source: keytable::source_path(),
            sysfs: PathBuf::from(akm_core::hid_params::SYSFS_DIR),
        }
    }
}

pub struct KeymapIface {
    paths: KeymapPaths,
    privileged: Arc<dyn Privileged>,
    /// Serialises the read-modify-write of keymap.toml.
    edit: Mutex<()>,
}

impl KeymapIface {
    pub fn new(paths: KeymapPaths, privileged: Arc<dyn Privileged>) -> Self {
        Self { paths, privileged, edit: Mutex::new(()) }
    }

    fn load(&self) -> zbus::fdo::Result<Keymap> {
        Keymap::load(&self.paths.keymap).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn edit(&self, profile: &str, f: impl FnOnce(&mut keymap::Profile) -> Result<(), String>) -> zbus::fdo::Result<String> {
        let _g = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        let mut km = self.load()?;
        let name = if profile.is_empty() { km.active.clone() } else { profile.to_string() };
        let p = km.profile_mut(&name).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        f(p).map_err(zbus::fdo::Error::InvalidArgs)?;
        km.save(&self.paths.keymap).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(keytable::keymap_json(&km, &self.paths.keymap).to_string())
    }

    fn table(&self, all: bool) -> String {
        let km = Keymap::load(&self.paths.keymap).unwrap_or_default();
        let st = HidState::read_in(&self.paths.sysfs);
        keytable::table_json(keytable::detect_pid(), &st, &keymap::read_installed(&self.paths.installed), &km, all).to_string()
    }
}

/// The work of `Apply()` (blocking: runs pkexec).
pub fn apply_blocking(paths: &KeymapPaths, privileged: &dyn Privileged) -> Result<String, String> {
    let km = Keymap::load(&paths.keymap).map_err(|e| e.to_string())?;
    let inst = keymap::read_installed(&paths.installed).unwrap_or_default();
    let plan = keytable::plan(&km, &HidState::read_in(&paths.sysfs), &inst).map_err(|e| e.to_string())?;
    if plan.is_empty() {
        return Ok(format!("nothing to do: profile {} is already applied", km.active));
    }
    let mut done = Vec::new();
    if let Some(h) = &plan.hwdb {
        keytable::write_source(&paths.source, h).map_err(|e| e.to_string())?;
        let r = privileged.run(KEYMAP_HELPER_PATH, &["install".into()]);
        let _ = std::fs::remove_file(&paths.source);
        r?;
        done.push("key mapping installed".to_string());
    }
    if plan.remove_hwdb {
        privileged.run(KEYMAP_HELPER_PATH, &["remove".into()])?;
        done.push("key mapping removed".to_string());
    }
    if !plan.params.is_empty() {
        let mut args = vec!["set-params".to_string()];
        args.extend(plan.params.iter().map(|(n, v)| format!("{n}={v}")));
        args.push("--persist".into());
        privileged.run(PARAM_HELPER_PATH, &args)?;
        done.push(format!("hid_apple {} (all Apple keyboards)", args[1..args.len() - 1].join(" ")));
    }
    Ok(done.join("; "))
}

fn failed(e: String) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e)
}

#[interface(name = "com.agenceapi.AppleKbMonitor1.Keymap")]
impl KeymapIface {
    fn key_table(&self, all: bool) -> String {
        self.table(all)
    }

    fn keymap(&self) -> zbus::fdo::Result<String> {
        Ok(keytable::keymap_json(&self.load()?, &self.paths.keymap).to_string())
    }

    fn set_key(&self, profile: &str, key: &str, code: &str) -> zbus::fdo::Result<String> {
        let sc = keymap::parse_key_ref(key).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let code = if code.is_empty() { None } else { Some(keymap::parse_code(code).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?) };
        self.edit(profile, |p| {
            match code {
                Some(c) => {
                    p.keys.insert(sc, c);
                }
                None => {
                    p.keys.remove(&sc);
                }
            }
            Ok(())
        })
    }

    fn set_preset(&self, profile: &str, preset: &str) -> zbus::fdo::Result<String> {
        let pr = Preset::parse(preset).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        self.edit(profile, |p| {
            p.preset = pr;
            Ok(())
        })
    }

    fn use_profile(&self, profile: &str) -> zbus::fdo::Result<String> {
        let _g = self.edit.lock().unwrap_or_else(|e| e.into_inner());
        let mut km = self.load()?;
        km.profile_mut(profile).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        km.active = profile.to_string();
        km.save(&self.paths.keymap).map_err(|e| failed(e.to_string()))?;
        Ok(keytable::keymap_json(&km, &self.paths.keymap).to_string())
    }

    async fn apply(&self) -> zbus::fdo::Result<String> {
        let (paths, pr) = (self.paths.clone(), self.privileged.clone());
        unblock(move || apply_blocking(&paths, pr.as_ref())).await.map_err(failed)
    }

    async fn reset(&self) -> zbus::fdo::Result<String> {
        let pr = self.privileged.clone();
        unblock(move || pr.run(KEYMAP_HELPER_PATH, &["remove".into()]).map(|_| "kernel key mapping restored".to_string()))
            .await
            .map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);
    impl Privileged for Rec {
        fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
            self.0.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            Ok(())
        }
    }

    fn paths(tag: &str) -> (PathBuf, KeymapPaths) {
        let d = std::env::temp_dir().join(format!("akmd-keymap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sys")).unwrap();
        for (n, v) in [("fnmode", "1"), ("iso_layout", "-1"), ("swap_opt_cmd", "0"), ("swap_ctrl_cmd", "0"), ("swap_fn_leftctrl", "0")] {
            std::fs::write(d.join("sys").join(n), format!("{v}\n")).unwrap();
        }
        let p = KeymapPaths {
            keymap: d.join("cfg/keymap.toml"),
            installed: d.join("etc/90-apple-kb-monitor.hwdb"),
            source: d.join("run/keymap.hwdb"),
            sysfs: d.join("sys"),
        };
        (d, p)
    }

    #[test]
    fn default_apply_runs_nothing() {
        let (d, p) = paths("default");
        let r = Rec::default();
        assert!(apply_blocking(&p, &r).unwrap().starts_with("nothing to do"));
        assert!(r.0.lock().unwrap().is_empty(), "default = no change, no authentication");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn edit_then_apply_calls_the_helpers() {
        let (d, p) = paths("edit");
        let r = Arc::new(Rec::default());
        let i = KeymapIface::new(p.clone(), r.clone());
        let j: serde_json::Value = serde_json::from_str(&i.set_key("", "F6", "KEY_F13").unwrap()).unwrap();
        assert_eq!(j["profiles"]["default"]["keys"]["F6"], "KEY_F13");
        assert!(i.set_key("", "F13", "KEY_F1").is_err());
        assert!(i.set_key("", "F1", "KEY_F1\nKEY_F2").is_err());
        assert!(i.set_key("../x", "F1", "KEY_F2").is_err());
        i.set_preset("", "linux-pc").unwrap();
        assert!(i.set_preset("", "mac").is_err());
        let msg = apply_blocking(&p, r.as_ref()).unwrap();
        assert!(msg.contains("installed") && msg.contains("swap_opt_cmd=1"), "{msg}");
        let calls = r.0.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                format!("{KEYMAP_HELPER_PATH} install"),
                format!("{PARAM_HELPER_PATH} set-params swap_opt_cmd=1 --persist"),
            ]
        );
        assert!(!p.source.exists(), "hand-over file removed after the helper");
        let t: serde_json::Value = serde_json::from_str(&i.key_table(false)).unwrap();
        assert_eq!(t["profile"], "default");
        assert!(i.use_profile("Bad").is_err());
        i.use_profile("work").unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn real_backend_refuses_other_programs() {
        assert!(Pkexec::default().run("/bin/sh", &[]).unwrap_err().contains("not allowed"));
        let src = include_str!("keymap.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("env::var"), "no env var picks a program");
    }
}
