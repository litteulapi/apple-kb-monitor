//! `akmctl keys`, `akmctl keymap ...`, `akmctl set|get param` (#247).
//!
//! The manual mapping is a udev hwdb file installed by
//! `akm-keymap-helper` (polkit `install-keymap`); the kernel parameters go
//! through `akm-helper set-params` (polkit `set-fnmode`). No grab, no uinput,
//! no keyd.

use std::path::Path;
use std::process::Command as Proc;

use akm_core::keymap::{self, HidState, Keymap, Preset};
use akm_core::keytable;

use crate::cli::{KeymapCmd, EXIT_ERROR, EXIT_OK};
use crate::kde;

/// Absolute paths: never resolved through `$PATH` (the programs run as root).
const PKEXEC: &str = "/usr/bin/pkexec";
pub const KEYMAP_HELPER: &str = "/usr/lib/apple-kb-monitor/akm-keymap-helper";
pub const PARAM_HELPER: &str = "/usr/lib/apple-kb-monitor/akm-helper";

fn fail(m: &str) -> u8 {
    eprintln!("akmctl: {m}");
    EXIT_ERROR
}

/// `pkexec <program> <args>`; the helper prints its own message.
fn pkexec(program: &str, args: &[String]) -> Result<(), String> {
    let st = Proc::new(PKEXEC).arg(program).args(args).status().map_err(|e| format!("cannot run pkexec: {e}"))?;
    match st.code() {
        Some(0) => Ok(()),
        Some(126) => Err("authentication dismissed or not authorized".into()),
        Some(127) => Err("authentication failed or helper not found".into()),
        Some(c) => Err(format!("{program} failed (exit {c})")),
        None => Err(format!("{program} killed by a signal")),
    }
}

fn session_bus() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::session().map_err(|e| format!("session bus: {e}"))
}

pub fn installed() -> Result<Vec<keymap::HwdbRecord>, keymap::KeymapError> {
    keymap::read_installed(Path::new(keymap::HWDB_PATH))
}

/// `akmctl keys`.
pub fn cmd_keys(check: bool, all: bool, json: bool) -> u8 {
    let km = match Keymap::load(&keymap::default_path()) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("akmctl: {e} (default profile used)");
            Keymap::default()
        }
    };
    let table = keytable::table_json(keytable::detect_pid(), &HidState::read(), &installed(), &km, all);
    let kde = session_bus().and_then(|c| crate::keys::resolve(&table, |q| kde::action_for(&c, q)));
    if json {
        let mut t = table;
        if let Ok(m) = &kde {
            let actions: serde_json::Map<String, serde_json::Value> = m
                .iter()
                .map(|(q, a)| {
                    let v = a.as_ref().map_or(serde_json::Value::Null, |a| {
                        serde_json::json!({"component": a.component, "action": a.action, "component_name": a.component_name, "action_name": a.action_name})
                    });
                    (q.to_string(), v)
                })
                .collect();
            t["kde_actions"] = actions.into();
        } else {
            t["kde_actions"] = serde_json::Value::Null;
        }
        println!("{t}");
    } else {
        print!("{}", crate::keys::to_text(&table, &kde, check));
    }
    EXIT_OK
}

fn load() -> Result<Keymap, String> {
    Keymap::load(&keymap::default_path()).map_err(|e| e.to_string())
}

fn save(km: &Keymap) -> Result<(), String> {
    km.save(&keymap::default_path()).map_err(|e| e.to_string())
}

fn show(km: &Keymap, hwdb: bool) -> String {
    let mut o = format!("File:     {}\nActive:   {}\n", keymap::default_path().display(), km.active);
    for (n, p) in &km.profiles {
        o.push_str(&format!(
            "\n[{n}]{} preset {}\n  models: {}\n",
            if *n == km.active { " *" } else { "" },
            p.preset.map_or("none (hid_apple parameters left as they are)".to_string(), |pr| format!("{} ({})", pr.name(), pr.title())),
            p.models.iter().map(|m| format!("05ac:{m:04x}")).collect::<Vec<_>>().join(", ")
        ));
        let params: Vec<String> = p.effective_params().iter().map(|(k, v)| format!("{k}={v}")).collect();
        o.push_str(&format!("  hid_apple: {}\n", if params.is_empty() { "unchanged".to_string() } else { params.join(" ") }));
        if p.keys.is_empty() {
            o.push_str("  keys: none (kernel mapping)\n");
        }
        for (sc, c) in &p.keys {
            o.push_str(&format!("  {:<10} (0x{sc:x}) -> {}\n", keymap::scancode_label(*sc), keymap::display_code(*c)));
        }
        if hwdb && *n == km.active {
            let recs = p.hwdb_records();
            o.push_str(&if recs.is_empty() { "  hwdb: nothing to install\n".to_string() } else { format!("\n{}", keymap::render_hwdb(n, &recs)) });
        }
    }
    o
}

pub fn run(cmd: KeymapCmd) -> u8 {
    match cmd {
        KeymapCmd::Show { hwdb, json } => match load() {
            Ok(km) => {
                if json {
                    println!("{}", keytable::keymap_json(&km, &keymap::default_path()));
                } else {
                    print!("{}", show(&km, hwdb));
                }
                EXIT_OK
            }
            Err(e) => fail(&e),
        },
        KeymapCmd::Set { key, code, profile } => edit(profile, |p| {
            p.keys.insert(key, code);
            Ok(format!("{} -> {}", keymap::scancode_label(key), keymap::display_code(code)))
        }),
        KeymapCmd::Unset { key, profile } => edit(profile, |p| match p.keys.remove(&key) {
            Some(_) => Ok(format!("{} back to the kernel mapping", keymap::scancode_label(key))),
            None => Err(format!("{} is not remapped in this profile", keymap::scancode_label(key))),
        }),
        KeymapCmd::Preset { name, profile } => edit(profile, |p| {
            let pr = if name == "none" { None } else { Some(Preset::parse(&name).map_err(|e| e.to_string())?) };
            p.preset = pr;
            Ok(match pr {
                Some(pr) => {
                    let ps: Vec<String> = pr.params().iter().map(|(n, v)| format!("{n}={v}")).collect();
                    format!("preset {} ({}): {}", pr.name(), pr.title(), ps.join(" "))
                }
                None => "no preset: hid_apple parameters left as they are".into(),
            })
        }),
        KeymapCmd::Use { profile } => {
            let mut km = match load() {
                Ok(k) => k,
                Err(e) => return fail(&e),
            };
            if let Err(e) = km.profile_mut(&profile) {
                return fail(&e.to_string());
            }
            km.active = profile.clone();
            match save(&km) {
                Ok(()) => {
                    println!("active profile: {profile} (not applied yet: akmctl keymap apply)");
                    EXIT_OK
                }
                Err(e) => fail(&e),
            }
        }
        KeymapCmd::Apply { dry_run } => apply(dry_run),
        KeymapCmd::Reset => reset(),
        KeymapCmd::Rollback => match pkexec(KEYMAP_HELPER, &["rollback".into()]) {
            Ok(()) => EXIT_OK,
            Err(e) => fail(&e),
        },
        KeymapCmd::KdeApply { dry_run, undo } => kde_apply(dry_run, undo),
    }
}

fn edit(profile: Option<String>, f: impl FnOnce(&mut keymap::Profile) -> Result<String, String>) -> u8 {
    let mut km = match load() {
        Ok(k) => k,
        Err(e) => return fail(&e),
    };
    let name = profile.unwrap_or_else(|| km.active.clone());
    let p = match km.profile_mut(&name) {
        Ok(p) => p,
        Err(e) => return fail(&e.to_string()),
    };
    let msg = match f(p) {
        Ok(m) => m,
        Err(e) => return fail(&e),
    };
    match save(&km) {
        Ok(()) => {
            println!("[{name}] {msg}\nsaved in {}; nothing changes until `akmctl keymap apply`", keymap::default_path().display());
            EXIT_OK
        }
        Err(e) => fail(&e),
    }
}

fn apply(dry_run: bool) -> u8 {
    let km = match load() {
        Ok(k) => k,
        Err(e) => return fail(&e),
    };
    let inst = installed().unwrap_or_else(|e| {
        eprintln!("akmctl: {e}: it will be replaced");
        Vec::new()
    });
    let state = HidState::read();
    let plan = match keytable::plan(&km, &state, &inst) {
        Ok(p) => p,
        Err(e) => return fail(&e.to_string()),
    };
    if plan.is_empty() {
        println!("Nothing to do: profile {} is already applied.", km.active);
        return EXIT_OK;
    }
    if let Some(h) = &plan.hwdb {
        println!("hwdb to install in {}:\n{h}", keymap::HWDB_PATH);
    }
    if plan.remove_hwdb {
        println!("hwdb {} to remove (profile without remapped key)", keymap::HWDB_PATH);
    }
    if !plan.params.is_empty() {
        let s: Vec<String> = plan.params.iter().map(|(n, v)| format!("{n}={v}")).collect();
        println!("hid_apple parameters to set (persistent, ALL Apple keyboards): {}", s.join(" "));
    }
    if dry_run {
        return EXIT_OK;
    }
    if let Some(h) = &plan.hwdb {
        let src = keytable::source_path();
        if let Err(e) = keytable::write_source(&src, h) {
            return fail(&e.to_string());
        }
        let r = pkexec(KEYMAP_HELPER, &["install".into()]);
        let _ = std::fs::remove_file(&src);
        if let Err(e) = r {
            return fail(&e);
        }
    }
    if plan.remove_hwdb {
        if let Err(e) = pkexec(KEYMAP_HELPER, &["remove".into()]) {
            return fail(&e);
        }
    }
    if !plan.params.is_empty() {
        let mut args: Vec<String> = vec!["set-params".into()];
        args.extend(plan.params.iter().map(|(n, v)| format!("{n}={v}")));
        args.push("--persist".into());
        if let Err(e) = pkexec(PARAM_HELPER, &args) {
            return fail(&e);
        }
    }
    println!("Applied. Check with: akmctl keys --check");
    EXIT_OK
}

fn reset() -> u8 {
    if let Err(e) = pkexec(KEYMAP_HELPER, &["remove".into()]) {
        return fail(&e);
    }
    // The active profile no longer asks for remapped keys (file kept as .bak).
    let path = keymap::default_path();
    if let Ok(mut km) = load() {
        let active = km.active.clone();
        if let Ok(p) = km.profile_mut(&active) {
            if !p.keys.is_empty() {
                let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
                p.keys.clear();
                if let Err(e) = save(&km) {
                    return fail(&e);
                }
                println!("keys of profile {active} cleared ({} kept as .bak)", path.display());
            }
        }
    }
    println!("Kernel key mapping restored. hid_apple parameters are unchanged (akmctl keymap preset apple && akmctl keymap apply).");
    EXIT_OK
}

fn kde_apply(dry_run: bool, undo: bool) -> u8 {
    let conn = match session_bus() {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let steps = match kde::apply_missing(&conn, !dry_run, undo) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{e} (KDE Plasma session required)")),
    };
    for (b, s) in steps {
        let verb = if dry_run { "would" } else { "did" };
        match s {
            kde::Step::AlreadyBound(a) => println!("{}: {} already bound to {} - left alone", b.key, b.qt_name, a.label()),
            kde::Step::Add { existing } => println!(
                "{}: {verb} add {} to {}/{} ({existing} existing key(s) kept) - {}",
                b.key, b.qt_name, b.component, b.action, b.why
            ),
            kde::Step::Removed => println!("{}: {verb} remove {} from {}/{}", b.key, b.qt_name, b.component, b.action),
            kde::Step::NotPresent => println!("{}: {} not bound by us - nothing to undo", b.key, b.qt_name),
        }
    }
    EXIT_OK
}

/// `akmctl set param NAME VALUE [--persist]`.
pub fn cmd_set_param(name: &str, value: i32, persist: bool) -> u8 {
    let mut args = vec!["set-params".to_string(), format!("{name}={value}")];
    if persist {
        args.push("--persist".into());
    }
    eprintln!("note: hid_apple.{name} applies to ALL Apple keyboards of this computer");
    if let Err(e) = pkexec(PARAM_HELPER, &args) {
        return fail(&e);
    }
    match HidState::read().get(name) {
        Some(v) if v == value => {
            println!("{name} = {v}");
            if !persist {
                eprintln!("note: not persistent across reboots (use --persist)");
            }
            EXIT_OK
        }
        Some(v) => fail(&format!("write accepted but {name} reads {v}, expected {value}")),
        None if persist => {
            println!("{name} = {value} saved; hid_apple is not loaded, applied at next module load");
            EXIT_OK
        }
        None => fail(&format!("cannot read {name} back")),
    }
}

/// `akmctl get param [NAME]`.
pub fn cmd_get_param(name: Option<&str>) -> u8 {
    let s = HidState::read();
    for p in keymap::KERNEL_PARAMS.iter().filter(|p| name.is_none_or(|n| n == p.name)) {
        let v = s.get(p.name).map_or("?".to_string(), |v| v.to_string());
        if name.is_some() {
            println!("{v}");
        } else {
            println!("{:<17} {v:>3}   {}", p.name, p.help);
        }
    }
    EXIT_OK
}

/// Clap value parsers.
pub fn parse_key(s: &str) -> Result<u32, String> {
    keymap::parse_key_ref(s).map_err(|e| e.to_string())
}

pub fn parse_code(s: &str) -> Result<u16, String> {
    keymap::parse_code(s).map_err(|e| e.to_string())
}

/// `apple`, `fkeys`, `linux-pc` or `none` (kept as text: `none` = no preset).
pub fn parse_preset(s: &str) -> Result<String, String> {
    if s == "none" {
        return Ok(s.into());
    }
    Preset::parse(s).map(|p| p.name().to_string()).map_err(|e| format!("{e}, or none"))
}

pub fn parse_profile(s: &str) -> Result<String, String> {
    if keymap::valid_profile_name(s) {
        Ok(s.to_string())
    } else {
        Err(format!("bad profile name {s:?} (a-z 0-9 - _, 1-32 characters)"))
    }
}

pub fn parse_param_name(s: &str) -> Result<String, String> {
    keymap::kernel_param(s).map(|p| p.name.to_string()).ok_or_else(|| {
        format!("unknown hid_apple parameter {s:?} ({})", keymap::KERNEL_PARAMS.iter().map(|p| p.name).collect::<Vec<_>>().join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_are_absolute_constants() {
        assert!(PKEXEC.starts_with('/') && KEYMAP_HELPER.starts_with("/usr/lib/apple-kb-monitor/"));
        let src = include_str!("keymapcmd.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("env::var"), "no env var picks a program");
    }

    #[test]
    fn show_default() {
        let s = show(&Keymap::default(), true);
        assert!(s.contains("[default] * preset none"), "{s}");
        assert!(s.contains("keys: none (kernel mapping)") && s.contains("hwdb: nothing to install"));
        assert!(s.contains("hid_apple: unchanged"));
    }
}
