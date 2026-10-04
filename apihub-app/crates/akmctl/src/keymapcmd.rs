//! `akmctl keys`, `akmctl keymap ...`, `akmctl set|get param`.

use akm_core::{tr, trn};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command as Proc;

use akm_core::keymap::{self, HidState, Keymap, Preset};
use akm_core::keytable::{self, HandOver};

use crate::cli::{KeymapCmd, EXIT_ERROR, EXIT_OK};
use crate::kde;

use akm_core::paths::{HELPER, PKEXEC};

fn fail(m: &str) -> u8 {
    eprintln!("akmctl: {m}");
    EXIT_ERROR
}

fn pkexec(program: &str, args: &[String]) -> Result<(), String> {
    let st = Proc::new(PKEXEC)
        .arg(program)
        .args(args)
        .status()
        .map_err(|e| tr!("cannot run pkexec: {e}", e = e))?;
    if let Some(e) = crate::pkexec::failure(st.code(), program) {
        return Err(e);
    }
    match st.code() {
        Some(0) => Ok(()),
        Some(c) => Err(tr!("{program} failed (exit {c})", program = program, c = c)),
        None => Err(tr!("{program} killed by a signal", program = program)),
    }
}

fn session_bus() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::session().map_err(|e| tr!("session bus: {e}", e = e))
}

pub fn installed() -> Result<Vec<keymap::HwdbRecord>, keymap::KeymapError> {
    keymap::read_installed(Path::new(keymap::HWDB_PATH))
}

pub fn cmd_keys(check: bool, all: bool, json: bool) -> u8 {
    let km = match Keymap::load(&keymap::default_path()) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{}", tr!("akmctl: {e} (default profile used)", e = e));
            Keymap::default()
        }
    };
    let table = keytable::table_json(
        keytable::detect_pid(),
        &HidState::read(),
        &installed(),
        &km,
        all,
    );
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
    let mut o = format!(
        "{:<10}{}\n{:<10}{}\n",
        tr!("File:"),
        keymap::default_path().display(),
        tr!("Active:"),
        km.active
    );
    for (n, p) in &km.profiles {
        let preset = p.preset.map_or_else(
            || tr!("none (hid_apple parameters left as they are)"),
            |pr| format!("{} ({})", pr.name(), akm_core::i18n::gettext(pr.title())),
        );
        let models = p
            .models
            .iter()
            .map(|m| format!("05ac:{m:04x}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            o,
            "\n[{n}]{} {}\n  {}",
            if *n == km.active { " *" } else { "" },
            tr!("preset {preset}", preset = preset),
            tr!("models: {models}", models = models)
        );
        let params: Vec<String> = p
            .effective_params()
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let _ = writeln!(
            o,
            "  hid_apple: {}",
            if params.is_empty() {
                tr!("unchanged")
            } else {
                params.join(" ")
            }
        );
        if p.keys.is_empty() {
            let _ = writeln!(o, "  {}", tr!("keys: none (kernel mapping)"));
        }
        for (sc, c) in &p.keys {
            let _ = writeln!(
                o,
                "  {:<10} (0x{sc:x}) -> {}",
                keymap::scancode_label(*sc),
                keymap::display_code(*c)
            );
        }
        if hwdb && *n == km.active {
            let recs = p.hwdb_records();
            if recs.is_empty() {
                let _ = writeln!(o, "  {}", tr!("hwdb: nothing to install"));
            } else {
                let _ = write!(o, "\n{}", keymap::render_hwdb(n, &recs));
            }
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
            Ok(format!(
                "{} -> {}",
                keymap::scancode_label(key),
                keymap::display_code(code)
            ))
        }),
        KeymapCmd::Unset { key, profile } => edit(profile, |p| match p.keys.remove(&key) {
            Some(_) => Ok(tr!(
                "{} back to the kernel mapping",
                keymap::scancode_label(key)
            )),
            None => Err(tr!(
                "{} is not remapped in this profile",
                keymap::scancode_label(key)
            )),
        }),
        KeymapCmd::Preset { name, profile } => edit(profile, |p| {
            let pr = if name == "none" {
                None
            } else {
                Some(Preset::parse(&name).map_err(|e| e.to_string())?)
            };
            p.preset = pr;
            Ok(match pr {
                Some(pr) => {
                    let ps: Vec<String> = pr
                        .params()
                        .iter()
                        .map(|(n, v)| format!("{n}={v}"))
                        .collect();
                    tr!(
                        "preset {} ({}): {}",
                        pr.name(),
                        akm_core::i18n::gettext(pr.title()),
                        ps.join(" ")
                    )
                }
                None => tr!("no preset: hid_apple parameters left as they are"),
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
            km.active.clone_from(&profile);
            match save(&km) {
                Ok(()) => {
                    println!(
                        "{}",
                        tr!(
                            "active profile: {profile} (not applied yet: akmctl keymap apply)",
                            profile = profile
                        )
                    );
                    EXIT_OK
                }
                Err(e) => fail(&e),
            }
        }
        KeymapCmd::Apply { dry_run } => apply(dry_run),
        KeymapCmd::Reset => reset(),
        KeymapCmd::Rollback => match handed_over(&keytable::source_path(), || {
            pkexec(HELPER, &["install-keymap".into(), "rollback".into()])
        }) {
            Ok(()) => EXIT_OK,
            Err(e) => fail(&e),
        },
        KeymapCmd::KdeApply { dry_run, undo } => kde_apply(dry_run, undo),
    }
}

fn edit(
    profile: Option<String>,
    f: impl FnOnce(&mut keymap::Profile) -> Result<String, String>,
) -> u8 {
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
            println!(
                "{}",
                tr!(
                    "[{name}] {msg}\nsaved in {path}; nothing changes until `akmctl keymap apply`",
                    name = name,
                    msg = msg,
                    path = keymap::default_path().display()
                )
            );
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
        eprintln!("{}", tr!("akmctl: {e}: it will be replaced", e = e));
        Vec::new()
    });
    let state = HidState::read();
    let plan = keytable::plan(&km, &state, &inst);
    if plan.is_empty() {
        println!(
            "{}",
            tr!("Nothing to do: profile {} is already applied.", km.active)
        );
        return EXIT_OK;
    }
    if let Some(h) = &plan.hwdb {
        println!(
            "{}",
            tr!(
                "hwdb to install in {path}:\n{h}",
                path = keymap::HWDB_PATH,
                h = h
            )
        );
    }
    if plan.remove_hwdb {
        println!(
            "{}",
            tr!(
                "hwdb {} to remove (profile without remapped key)",
                keymap::HWDB_PATH
            )
        );
    }
    if !plan.params.is_empty() {
        let s: Vec<String> = plan
            .params
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        println!(
            "{}",
            tr!(
                "hid_apple parameters to set (persistent, ALL Apple keyboards): {}",
                s.join(" ")
            )
        );
    }
    if dry_run {
        return EXIT_OK;
    }
    let hand = match HandOver::lock(&keytable::source_path()) {
        Ok(h) => h,
        Err(e) => return fail(&e.to_string()),
    };
    if let Some(h) = &plan.hwdb {
        if let Err(e) = hand.with_hwdb(h, || {
            pkexec(HELPER, &["install-keymap".into(), "install".into()])
        }) {
            return fail(&e);
        }
    }
    if plan.remove_hwdb {
        if let Err(e) = pkexec(HELPER, &["install-keymap".into(), "remove".into()]) {
            return fail(&e);
        }
    }
    if !plan.params.is_empty() {
        let mut args: Vec<String> = vec!["set-fnmode".into()];
        args.extend(plan.params.iter().map(|(n, v)| format!("{n}={v}")));
        args.push("--persist".into());
        if let Err(e) = pkexec(HELPER, &args) {
            return fail(&e);
        }
    }
    println!("{}", tr!("Applied. Check with: akmctl keys --check"));
    EXIT_OK
}

/// Runs `run` under the keymap hand-over lock, so no apply or reset of the daemon interleaves.
fn handed_over(
    source: &std::path::Path,
    run: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let _hand = HandOver::lock(source).map_err(|e| e.to_string())?;
    run()
}

fn reset() -> u8 {
    let hand = match HandOver::lock(&keytable::source_path()) {
        Ok(h) => h,
        Err(e) => return fail(&e.to_string()),
    };
    let removed = pkexec(HELPER, &["install-keymap".into(), "remove".into()]);
    drop(hand);
    if let Err(e) = removed {
        return fail(&e);
    }
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
                println!(
                    "{}",
                    tr!(
                        "keys of profile {active} cleared ({path} kept as .bak)",
                        active = active,
                        path = path.display()
                    )
                );
            }
        }
    }
    println!("{}", tr!("Kernel key mapping restored. hid_apple parameters are unchanged (akmctl keymap preset apple && akmctl keymap apply)."));
    EXIT_OK
}

fn kde_apply(dry_run: bool, undo: bool) -> u8 {
    let conn = match session_bus() {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let steps = match kde::apply_missing(&conn, !dry_run, undo) {
        Ok(s) => s,
        Err(e) => return fail(&tr!("{e} (KDE Plasma session required)", e = e)),
    };
    for (b, s) in steps {
        match s {
            kde::Step::AlreadyBound(a) => println!(
                "{}: {}",
                b.key,
                tr!(
                    "{shortcut} already bound to {action} - left alone",
                    shortcut = b.qt_name,
                    action = a.label()
                )
            ),
            kde::Step::Add { existing } => {
                let target = format!("{}/{}", b.component, b.action);
                let text = if dry_run {
                    trn!(
                        "would add {shortcut} to {target} ({existing} existing key kept) - {why}",
                        "would add {shortcut} to {target} ({existing} existing keys kept) - {why}",
                        existing,
                        shortcut = b.qt_name,
                        target = target,
                        existing = existing,
                        why = b.why
                    )
                } else {
                    trn!(
                        "added {shortcut} to {target} ({existing} existing key kept) - {why}",
                        "added {shortcut} to {target} ({existing} existing keys kept) - {why}",
                        existing,
                        shortcut = b.qt_name,
                        target = target,
                        existing = existing,
                        why = b.why
                    )
                };
                println!("{}: {text}", b.key);
            }
            kde::Step::Removed => {
                let target = format!("{}/{}", b.component, b.action);
                let text = if dry_run {
                    tr!(
                        "would remove {shortcut} from {target}",
                        shortcut = b.qt_name,
                        target = target
                    )
                } else {
                    tr!(
                        "removed {shortcut} from {target}",
                        shortcut = b.qt_name,
                        target = target
                    )
                };
                println!("{}: {text}", b.key);
            }
            kde::Step::NotPresent => println!(
                "{}: {}",
                b.key,
                tr!(
                    "{shortcut} not bound by us - nothing to undo",
                    shortcut = b.qt_name
                )
            ),
        }
    }
    EXIT_OK
}

pub fn cmd_set_param(name: &str, value: i32, persist: bool) -> u8 {
    let mut args = vec!["set-fnmode".to_string(), format!("{name}={value}")];
    if persist {
        args.push("--persist".into());
    }
    eprintln!(
        "{}",
        tr!(
            "note: hid_apple.{name} applies to ALL Apple keyboards of this computer",
            name = name
        )
    );
    if let Err(e) = pkexec(HELPER, &args) {
        return fail(&e);
    }
    match HidState::read().get(name) {
        Some(v) if v == value => {
            println!("{name} = {v}");
            if !persist {
                eprintln!(
                    "{}",
                    tr!("note: not persistent across reboots (use --persist)")
                );
            }
            EXIT_OK
        }
        Some(v) => fail(&tr!(
            "write accepted but {name} reads {v}, expected {value}",
            name = name,
            v = v,
            value = value
        )),
        None if persist => {
            println!(
                "{}",
                tr!(
                    "{name} = {value} saved; hid_apple is not loaded, applied at next module load",
                    name = name,
                    value = value
                )
            );
            EXIT_OK
        }
        None => fail(&tr!("cannot read {name} back", name = name)),
    }
}

pub fn cmd_get_param(name: Option<&str>) -> u8 {
    let s = HidState::read();
    for p in keymap::KERNEL_PARAMS
        .iter()
        .filter(|p| name.is_none_or(|n| n == p.name))
    {
        let v = s.get(p.name).map_or("?".to_string(), |v| v.to_string());
        if name.is_some() {
            println!("{v}");
        } else {
            println!("{:<17} {v:>3}   {}", p.name, p.help);
        }
    }
    EXIT_OK
}

pub fn parse_key(s: &str) -> Result<u32, String> {
    keymap::parse_key_ref(s).map_err(|e| e.to_string())
}

pub fn parse_code(s: &str) -> Result<u16, String> {
    keymap::parse_code(s).map_err(|e| e.to_string())
}

pub fn parse_preset(s: &str) -> Result<String, String> {
    if s == "none" {
        return Ok(s.into());
    }
    Preset::parse(s)
        .map(|p| p.name().to_string())
        .map_err(|e| format!("{e}, or none"))
}

pub fn parse_profile(s: &str) -> Result<String, String> {
    if keymap::valid_profile_name(s) {
        Ok(s.to_string())
    } else {
        Err(tr!(
            "bad profile name {s} (a-z 0-9 - _, 1-32 characters)",
            s = format!("{:?}", s)
        ))
    }
}

pub fn parse_param_name(s: &str) -> Result<String, String> {
    keymap::kernel_param(s)
        .map(|p| p.name.to_string())
        .ok_or_else(|| {
            tr!(
                "unknown hid_apple parameter {s} ({known})",
                s = format!("{:?}", s),
                known = keymap::KERNEL_PARAMS
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_runs_nothing_while_the_hand_over_lock_is_held() {
        let d = std::env::temp_dir().join(format!("akm-rollback-{}", std::process::id()));
        let source = d.join("hand-over.hwdb");
        let held = HandOver::lock(&source).unwrap();
        let mut ran = false;
        assert!(handed_over(&source, || {
            ran = true;
            Ok(())
        })
        .is_err());
        assert!(!ran);
        drop(held);
        assert!(handed_over(&source, || {
            ran = true;
            Ok(())
        })
        .is_ok());
        assert!(ran);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn helpers_are_absolute_constants() {
        assert!(PKEXEC.starts_with('/') && HELPER.starts_with("/usr/lib/apple-kb-monitor/"));
        let src = include_str!("keymapcmd.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("env::var"), "no env var picks a program");
    }

    #[test]
    fn show_default() {
        let s = show(&Keymap::default(), true);
        assert!(s.contains("[default] * preset none"), "{s}");
        assert!(
            s.contains("keys: none (kernel mapping)") && s.contains("hwdb: nothing to install")
        );
        assert!(s.contains("hid_apple: unchanged"));
    }
}
