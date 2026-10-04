//! D-Bus interface `com.agenceapi.AppleKbMonitor1.Keymap` on the root object.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use akm_core::keymap::{self, HidState, Keymap, Preset};
use akm_core::keytable::{self, HandOver};
use zbus::interface;

use crate::devices::unblock;
use crate::privileged::{self, Busy, Flight};
use akm_core::paths::{HELPER, PKEXEC};

/// Privileged side, replaceable in tests.
pub trait Privileged: Send + Sync {
    /// # Errors
    /// Why the privileged program failed or was refused.
    fn run(&self, program: &str, args: &[String]) -> Result<(), String>;

    /// Runs the helper with `args` while the hand-over file of `hand` holds `hwdb`.
    ///
    /// # Errors
    /// Why the file could not be written, or why the helper failed or was refused.
    fn run_with_source(&self, hand: &HandOver, hwdb: &str, args: &[String]) -> Result<(), String> {
        hand.with_hwdb(hwdb, || self.run(HELPER, args))
    }
}

/// pkexec with constant programs, under the process-wide single flight + cool-down.
#[derive(Debug)]
pub struct Pkexec {
    flight: Flight,
}

impl Default for Pkexec {
    fn default() -> Self {
        Self {
            flight: Flight::shared(),
        }
    }
}

impl Pkexec {
    /// `f` under the flight guard: a refused caller has touched nothing.
    fn in_flight(&self, f: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        self.flight.acquire().map_err(|b| match b {
            Busy::Pending => tr!("an authentication is already pending"),
            Busy::CoolDown(_) => tr!("too many requests, retry shortly"),
        })?;
        let r = f();
        self.flight.release();
        r
    }
}

fn check_helper(program: &str) -> Result<(), String> {
    if program != HELPER {
        return Err(format!("program not allowed: {program}"));
    }
    if !Path::new(program).is_file() {
        return Err(tr!(
            "privileged helper not installed ({program})",
            program = program
        ));
    }
    Ok(())
}

impl Privileged for Pkexec {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        check_helper(program)?;
        self.in_flight(|| privileged::run(Path::new(PKEXEC), Path::new(program), args))
    }

    fn run_with_source(&self, hand: &HandOver, hwdb: &str, args: &[String]) -> Result<(), String> {
        self.in_flight(|| {
            check_helper(HELPER)?;
            hand.with_hwdb(hwdb, || {
                privileged::run(Path::new(PKEXEC), Path::new(HELPER), args)
            })
        })
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
    edit: Mutex<()>,
}

impl KeymapIface {
    pub fn new(paths: KeymapPaths, privileged: Arc<dyn Privileged>) -> Self {
        Self {
            paths,
            privileged,
            edit: Mutex::new(()),
        }
    }

    fn load(&self) -> zbus::fdo::Result<Keymap> {
        Keymap::load(&self.paths.keymap).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn edit(
        &self,
        profile: &str,
        f: impl FnOnce(&mut keymap::Profile) -> Result<(), String>,
    ) -> zbus::fdo::Result<String> {
        let _g = self
            .edit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut km = self.load()?;
        let name = if profile.is_empty() {
            km.active.clone()
        } else {
            profile.to_string()
        };
        let p = km
            .profile_mut(&name)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        f(p).map_err(zbus::fdo::Error::InvalidArgs)?;
        km.save(&self.paths.keymap)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(keytable::keymap_json(&km, &self.paths.keymap).to_string())
    }

    fn table(&self, all: bool) -> String {
        let km = Keymap::load(&self.paths.keymap).unwrap_or_default();
        let st = HidState::read_in(&self.paths.sysfs);
        keytable::table_json(
            keytable::detect_pid(),
            &st,
            &keymap::read_installed(&self.paths.installed),
            &km,
            all,
        )
        .to_string()
    }
}

/// The work of `Reset()` (blocking: runs pkexec). No detail on success: the
/// caller's own headline says it, in the user's language.
///
/// # Errors
/// Why the key mapping could not be removed.
pub fn reset_blocking(source: &Path, privileged: &dyn Privileged) -> Result<String, String> {
    // Taken before the flight guard: a refused lock must not start the cool-down.
    let _hand = HandOver::lock(source).map_err(|e| e.to_string())?;
    privileged
        .run(HELPER, &["install-keymap".into(), "remove".into()])
        .map(|()| String::new())
}

/// The work of `Apply()` (blocking: runs pkexec).
///
/// # Errors
/// Why the keymap could not be built or installed.
pub fn apply_blocking(paths: &KeymapPaths, privileged: &dyn Privileged) -> Result<String, String> {
    let km = Keymap::load(&paths.keymap).map_err(|e| e.to_string())?;
    let inst = keymap::read_installed(&paths.installed).unwrap_or_default();
    let plan = keytable::plan(&km, &HidState::read_in(&paths.sysfs), &inst);
    if plan.is_empty() {
        return Ok(tr!(
            "Nothing to do: profile {} is already applied.",
            km.active
        ));
    }
    let hand = HandOver::lock(&paths.source).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    if let Some(h) = &plan.hwdb {
        privileged.run_with_source(&hand, h, &["install-keymap".into(), "install".into()])?;
        done.push(tr!("key mapping installed"));
    }
    if plan.remove_hwdb {
        privileged.run(HELPER, &["install-keymap".into(), "remove".into()])?;
        done.push(tr!("key mapping removed"));
    }
    if !plan.params.is_empty() {
        let mut args = vec!["set-fnmode".to_string()];
        args.extend(plan.params.iter().map(|(n, v)| format!("{n}={v}")));
        args.push("--persist".into());
        privileged.run(HELPER, &args)?;
        done.push(tr!(
            "hid_apple {params} (all Apple keyboards)",
            params = args[1..args.len() - 1].join(" ")
        ));
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
        let sc =
            keymap::parse_key_ref(key).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let code = if code.is_empty() {
            None
        } else {
            Some(
                keymap::parse_code(code)
                    .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?,
            )
        };
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
        // "" or "none" = no preset (parameters left as they are)
        let pr = match preset {
            "" | "none" => None,
            p => Some(Preset::parse(p).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?),
        };
        self.edit(profile, |p| {
            p.preset = pr;
            Ok(())
        })
    }

    fn use_profile(&self, profile: &str) -> zbus::fdo::Result<String> {
        let _g = self
            .edit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut km = self.load()?;
        km.profile_mut(profile)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        km.active = profile.to_string();
        km.save(&self.paths.keymap)
            .map_err(|e| failed(e.to_string()))?;
        Ok(keytable::keymap_json(&km, &self.paths.keymap).to_string())
    }

    async fn apply(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        crate::devices::caller_uid(conn, &hdr).await?;
        let (paths, pr) = (self.paths.clone(), self.privileged.clone());
        let r = unblock(move || apply_blocking(&paths, pr.as_ref()))
            .await?
            .map_err(failed)?;
        crate::service::emit_keymap_params(conn).await;
        Ok(r)
    }

    async fn reset(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        crate::devices::caller_uid(conn, &hdr).await?;
        let (source, pr) = (self.paths.source.clone(), self.privileged.clone());
        let r = unblock(move || reset_blocking(&source, pr.as_ref()))
            .await?
            .map_err(failed)?;
        crate::service::emit_keymap_params(conn).await;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pkexec_method_checks_the_callers_uid() {
        let keymap = akm_core::srclint::prod_tokens(include_str!("keymap.rs"));
        let settings = akm_core::srclint::prod_tokens(include_str!("config_api.rs"));
        for (code, m, end) in [
            (&keymap, "asyncfnapply(", "unblock("),
            (&keymap, "asyncfnreset(", "unblock("),
            (&settings, "asyncfnrun_akmctl(", "allowed(&args)"),
        ] {
            let body = &code[code.find(m).unwrap()..];
            let body = &body[..body.find(end).unwrap()];
            assert!(
                body.contains("crate::devices::caller_uid(conn,&hdr).await?"),
                "{m}"
            );
        }
    }

    #[test]
    fn a_refused_apply_leaves_the_pending_hand_over_file_alone() {
        let d = std::env::temp_dir().join(format!("akmd-keymap-flight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let source = d.join("keymap.hwdb");
        let pk = Pkexec {
            flight: Flight::default(),
        };
        keytable::write_source(&source, "first").unwrap();
        pk.flight.acquire().unwrap();
        let hand = HandOver::lock(&source).unwrap();
        let e = pk
            .run_with_source(&hand, "second", &["install-keymap".into()])
            .unwrap_err();
        assert!(e.contains("pending"), "{e}");
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "first");
        pk.flight.release();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);
    impl Privileged for Rec {
        fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            Ok(())
        }
    }

    fn paths(tag: &str) -> (PathBuf, KeymapPaths) {
        let d = std::env::temp_dir().join(format!("akmd-keymap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sys")).unwrap();
        for (n, v) in [
            ("fnmode", "1"),
            ("iso_layout", "-1"),
            ("swap_opt_cmd", "0"),
            ("swap_ctrl_cmd", "0"),
            ("swap_fn_leftctrl", "0"),
        ] {
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
    fn reset_removes_the_mapping_without_an_english_detail() {
        let (d, p) = paths("reset");
        let r = Rec::default();
        assert_eq!(reset_blocking(&p.source, &r).unwrap(), "");
        assert_eq!(r.0.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_held_hand_over_lock_blocks_reset_before_any_helper_run() {
        let (d, p) = paths("reset-locked");
        let held = HandOver::lock(&p.source).unwrap();
        let r = Rec::default();
        assert!(reset_blocking(&p.source, &r).is_err());
        assert!(r.0.lock().unwrap().is_empty());
        let pk = Pkexec {
            flight: Flight::default(),
        };
        assert!(reset_blocking(&p.source, &pk).is_err());
        assert!(
            pk.flight.acquire().is_ok(),
            "a refused lock starts no cool-down"
        );
        pk.flight.release();
        drop(held);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn default_apply_runs_nothing() {
        let (d, p) = paths("default");
        let r = Rec::default();
        assert!(apply_blocking(&p, &r).unwrap().starts_with("Nothing to do"));
        assert!(
            r.0.lock().unwrap().is_empty(),
            "default = no change, no authentication"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[allow(clippy::many_single_char_names)] // test fixtures with short local names
    #[test]
    fn edit_then_apply_calls_the_helpers() {
        let (d, p) = paths("edit");
        let r = Arc::new(Rec::default());
        let i = KeymapIface::new(p.clone(), r.clone());
        let j: serde_json::Value =
            serde_json::from_str(&i.set_key("", "F6", "KEY_F13").unwrap()).unwrap();
        assert_eq!(j["profiles"]["default"]["keys"]["F6"], "KEY_F13");
        assert!(i.set_key("", "F13", "KEY_F1").is_err());
        assert!(i.set_key("", "F1", "KEY_F1\nKEY_F2").is_err());
        assert!(i.set_key("../x", "F1", "KEY_F2").is_err());
        i.set_preset("", "linux-pc").unwrap();
        assert!(i.set_preset("", "mac").is_err());
        let msg = apply_blocking(&p, r.as_ref()).unwrap();
        assert!(
            msg.contains("installed") && msg.contains("swap_opt_cmd=1"),
            "{msg}"
        );
        let calls = r.0.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                format!("{HELPER} install-keymap install"),
                format!("{HELPER} set-fnmode swap_opt_cmd=1 --persist"),
            ]
        );
        assert!(
            !p.source.exists(),
            "hand-over file removed after the helper"
        );
        let t: serde_json::Value = serde_json::from_str(&i.key_table(false)).unwrap();
        assert_eq!(t["profile"], "default");
        assert!(i.use_profile("Bad").is_err());
        i.use_profile("work").unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn real_backend_refuses_other_programs() {
        assert!(Pkexec::default()
            .run("/bin/sh", &[])
            .unwrap_err()
            .contains("not allowed"));
        let src = include_str!("keymap.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("env::var"), "no env var picks a program");
    }
}
