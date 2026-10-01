//! `akmctl keys [--check] [--all] [--json]`: effective table of the special
//! keys for the connected keyboard — physical key → evdev code (after hwdb
//! and hid_apple, per fnmode) → xkb keysym / Qt key → KDE global shortcut.
//! Never reads a key press: the KDE side is asked to KGlobalAccel which
//! action a Qt key WOULD fire (#247).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::kde::Action;

/// KDE action per Qt key (`None` inside = nothing bound; outer `Err` = KDE
/// not reachable).
pub type Kde = Result<BTreeMap<u64, Option<Action>>, String>;

/// Every Qt key of the table, resolved once.
pub fn resolve(table: &Value, lookup: impl Fn(u32) -> Result<Option<Action>, String>) -> Kde {
    let mut m = BTreeMap::new();
    for r in table["rows"].as_array().into_iter().flatten() {
        for side in ["plain", "fn"] {
            if let Some(q) = r[side]["qt_key"].as_u64() {
                if let std::collections::btree_map::Entry::Vacant(e) = m.entry(q) {
                    e.insert(lookup(q as u32)?);
                }
            }
        }
    }
    Ok(m)
}

/// What the tester should observe when no KDE shortcut is bound.
fn unbound_hint(code: &str) -> &'static str {
    match code {
        "KEY_ALL_APPLICATIONS" => "nothing in KDE (Launch (D) unbound) - `akmctl keymap kde-apply` binds it to the application launcher",
        "KEY_EJECTCD" => "nothing in KDE (no shortcut uses Eject); remappable: akmctl keymap set Eject KEY_...",
        "KEY_NUMLOCK" => "NumLock toggles (handled by KWin, not a shortcut): J K L U I O M 7 8 9 turn into keypad keys until pressed again",
        "KEY_KBDILLUMDOWN" | "KEY_KBDILLUMUP" | "KEY_KBDILLUMTOGGLE" => "PowerDevil keyboard backlight: the A1314 has none, nothing visible",
        c if c.starts_with("KEY_F") => "sent to the focused application (e.g. F5 reload, F11 full screen in a browser)",
        "KEY_DELETE" | "KEY_INSERT" | "KEY_PAGEUP" | "KEY_PAGEDOWN" | "KEY_HOME" | "KEY_END" => "sent to the focused application",
        _ => "no KDE shortcut",
    }
}

/// Codes meant for the focused application, not for a KDE shortcut.
fn is_app_key(code: &str) -> bool {
    (code.starts_with("KEY_F") && code[5..].bytes().all(|b| b.is_ascii_digit()))
        || matches!(code, "KEY_DELETE" | "KEY_INSERT" | "KEY_PAGEUP" | "KEY_PAGEDOWN" | "KEY_HOME" | "KEY_END")
}

fn kde_of(kde: &Kde, side: &Value) -> (Option<String>, bool) {
    let code = side["code"].as_str().unwrap_or("?");
    match (kde, side["qt_key"].as_u64()) {
        (Ok(m), Some(q)) => match m.get(&q).cloned().flatten() {
            Some(a) => (Some(a.label()), true),
            None => (Some(unbound_hint(code).to_string()), false),
        },
        (Ok(_), None) => (Some(unbound_hint(code).to_string()), false),
        (Err(_), _) => (None, false),
    }
}

fn param(t: &Value, n: &str) -> String {
    t["params"][n].as_i64().map_or("?".into(), |v| v.to_string())
}

pub fn to_text(t: &Value, kde: &Kde, check: bool) -> String {
    let mut o = String::new();
    let pid = t["pid"].as_str().unwrap_or("?");
    o.push_str(&format!(
        "Keyboard:   05ac:{pid}{}  (Fn table: {})\n",
        if t["connected"].as_bool() == Some(true) { "" } else { " - not connected, table of the default model" },
        t["fn_table"].as_str().unwrap_or("not modelled: Fn layer unknown")
    ));
    o.push_str(&format!(
        "hid_apple:  fnmode={} iso_layout={} swap_opt_cmd={} swap_ctrl_cmd={} swap_fn_leftctrl={}{}\n",
        param(t, "fnmode"),
        param(t, "iso_layout"),
        param(t, "swap_opt_cmd"),
        param(t, "swap_ctrl_cmd"),
        param(t, "swap_fn_leftctrl"),
        if t["module_loaded"].as_bool() == Some(true) { "" } else { "  (hid_apple not loaded)" }
    ));
    let inst = &t["installed"];
    o.push_str(&match inst["error"].as_str() {
        Some(e) => format!("hwdb:       {e}\n"),
        None if inst["present"].as_bool() == Some(true) => format!("hwdb:       {} ({} key(s) remapped)\n", inst["path"].as_str().unwrap_or(""), inst["overrides"]),
        None => "hwdb:       none (kernel mapping)\n".into(),
    });
    o.push_str(&format!(
        "Profile:    {} (preset {}){}\n",
        t["profile"].as_str().unwrap_or("?"),
        t["preset"].as_str().unwrap_or("?"),
        if t["pending"].as_bool() == Some(true) { " - NOT applied yet: akmctl keymap apply" } else { "" }
    ));
    if let Err(e) = kde {
        o.push_str(&format!("KDE:        not reachable ({e}); actions not shown\n"));
    }
    o.push('\n');
    let mut missing = 0;
    for r in t["rows"].as_array().into_iter().flatten() {
        let key = r["key"].as_str().unwrap_or("?");
        let legend = r["legend"].as_str().unwrap_or("");
        let remap = if r["remapped"].as_bool() == Some(true) { format!("  [hwdb: {}]", r["base"].as_str().unwrap_or("?")) } else { String::new() };
        o.push_str(&format!("{key:<6} {legend}{remap}\n"));
        for (label, side) in [("key", &r["plain"]), ("Fn+key", &r["fn"])] {
            let code = side["code"].as_str().unwrap_or("?");
            let sym = side["keysym"].as_str().unwrap_or("-");
            let (action, bound) = kde_of(kde, side);
            let to_app = is_app_key(code);
            let mark = match (check, bound, to_app) {
                (false, ..) => "",
                (true, true, _) => "[ok]   ",
                (true, false, true) => "[app]  ",
                (true, false, false) => "[--]   ",
            };
            if check && !bound && label == "key" && !to_app {
                missing += 1;
            }
            o.push_str(&format!("   {mark}{label:<7} {code:<22} {sym:<22} {}\n", action.unwrap_or_default()));
        }
        if let Some(n) = r["note"].as_str() {
            o.push_str(&format!("          note: {n}\n"));
        }
    }
    if check {
        o.push_str(&format!(
            "\n{missing} legend key(s) without a KDE shortcut. Nothing was pressed or read: press each key and tick docs/TOUCHES.md \"A tester\".\n"
        ));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::keymap::{HidState, Keymap};
    use akm_core::keytable::table_json;

    fn table() -> Value {
        let p = HidState { fnmode: Some(1), ..HidState::KERNEL_DEFAULT };
        table_json(Some(0x0256), &p, &Ok(vec![]), &Keymap::default(), false)
    }

    fn act(c: &str, a: &str) -> Option<Action> {
        Some(Action { component: c.into(), action: a.into(), component_name: c.into(), action_name: a.into() })
    }

    /// The bindings measured on the reference session (2026-10-01).
    fn measured(q: u32) -> Result<Option<Action>, String> {
        Ok(match q {
            0x0100_00b3 => act("org_kde_powerdevil", "Decrease Screen Brightness"),
            0x0100_00b2 => act("org_kde_powerdevil", "Increase Screen Brightness"),
            0x0100_00ae => act("kwin", "ExposeAll"),
            0x0100_0082 => act("mediacontrol", "previousmedia"),
            0x0100_0080 => act("mediacontrol", "playpausemedia"),
            0x0100_0083 => act("mediacontrol", "nextmedia"),
            0x0100_0071 => act("kmix", "mute"),
            0x0100_0070 => act("kmix", "decrease_volume"),
            0x0100_0072 => act("kmix", "increase_volume"),
            _ => None,
        })
    }

    #[test]
    fn text_table_and_check() {
        let t = table();
        let kde = resolve(&t, measured);
        let s = to_text(&t, &kde, true);
        assert!(s.contains("[ok]   key     KEY_BRIGHTNESSDOWN     XF86MonBrightnessDown  Decrease Screen Brightness"), "{s}");
        assert!(s.contains("[--]   key     KEY_ALL_APPLICATIONS"), "{s}");
        assert!(s.contains("[app]  Fn+key  KEY_F1 "), "{s}");
        assert!(s.contains("KEY_NUMLOCK") && s.contains("keypad"));
        assert!(s.contains("3 legend key(s) without a KDE shortcut"), "F4, F6, Eject: {s}");
        // KDE absent: table still printed, no verdict claimed
        let s = to_text(&t, &Err("no session bus".into()), false);
        assert!(s.contains("KDE:        not reachable") && s.contains("KEY_VOLUMEUP"));
    }
}
