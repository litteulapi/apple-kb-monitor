//! `akmctl keys [--check] [--all] [--json]`.

use akm_core::{tr, trn};
use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::kde::Action;

pub type Kde = Result<BTreeMap<u64, Option<Action>>, String>;

pub fn resolve(table: &Value, lookup: impl Fn(i32) -> Result<Option<Action>, String>) -> Kde {
    let mut m = BTreeMap::new();
    for r in table["rows"].as_array().into_iter().flatten() {
        for side in ["plain", "fn"] {
            if let Some(q) = r[side]["qt_key"].as_u64() {
                let Ok(key) = i32::try_from(q) else { continue };
                if let std::collections::btree_map::Entry::Vacant(e) = m.entry(q) {
                    e.insert(lookup(key)?);
                }
            }
        }
    }
    Ok(m)
}

fn unbound_hint(code: &str) -> String {
    match code {
        "KEY_ALL_APPLICATIONS" => tr!("nothing in KDE (Launch (D) unbound) - `akmctl keymap kde-apply` binds it to the application launcher"),
        "KEY_EJECTCD" => tr!("nothing in KDE (no shortcut uses Eject); remappable: akmctl keymap set Eject KEY_..."),
        "KEY_NUMLOCK" => tr!("NumLock toggles (handled by KWin, not a shortcut): J K L U I O M 7 8 9 turn into keypad keys until pressed again"),
        "KEY_KBDILLUMDOWN" | "KEY_KBDILLUMUP" | "KEY_KBDILLUMTOGGLE" => tr!("PowerDevil keyboard backlight: the A1314 has none, nothing visible"),
        c if c.starts_with("KEY_F") => tr!("sent to the focused application (e.g. F5 reload, F11 full screen in a browser)"),
        "KEY_DELETE" | "KEY_INSERT" | "KEY_PAGEUP" | "KEY_PAGEDOWN" | "KEY_HOME" | "KEY_END" => tr!("sent to the focused application"),
        _ => tr!("no KDE shortcut"),
    }
}

fn is_app_key(code: &str) -> bool {
    (code.starts_with("KEY_F") && code[5..].bytes().all(|b| b.is_ascii_digit()))
        || matches!(
            code,
            "KEY_DELETE" | "KEY_INSERT" | "KEY_PAGEUP" | "KEY_PAGEDOWN" | "KEY_HOME" | "KEY_END"
        )
}

fn kde_of(kde: &Kde, side: &Value) -> (Option<String>, bool) {
    let code = side["code"].as_str().unwrap_or("?");
    match (kde, side["qt_key"].as_u64()) {
        (Ok(m), Some(q)) => match m.get(&q).cloned().flatten() {
            Some(a) => (Some(a.label()), true),
            None => (Some(unbound_hint(code).clone()), false),
        },
        (Ok(_), None) => (Some(unbound_hint(code).clone()), false),
        (Err(_), _) => (None, false),
    }
}

fn param(t: &Value, n: &str) -> String {
    t["params"][n]
        .as_i64()
        .map_or("?".into(), |v| v.to_string())
}

#[allow(clippy::too_many_lines)] // text rendered section by section
pub fn to_text(t: &Value, kde: &Kde, check: bool) -> String {
    let mut o = String::new();
    let pid = t["pid"].as_str().unwrap_or("?");
    let row = |k: String, v: String| format!("{k:<11} {v}\n");
    let model = if t["connected"].as_bool() == Some(true) {
        String::new()
    } else {
        tr!(" - not connected, table of the default model")
    };
    let fn_table = t["fn_table"]
        .as_str()
        .map_or_else(|| tr!("not modelled: Fn layer unknown"), str::to_string);
    o.push_str(&row(
        tr!("Keyboard:"),
        tr!(
            "05ac:{pid}{model}  (Fn table: {table})",
            pid = pid,
            model = model,
            table = fn_table
        ),
    ));
    let _ = writeln!(o,
        "hid_apple:  fnmode={} iso_layout={} swap_opt_cmd={} swap_ctrl_cmd={} swap_fn_leftctrl={}{}",
        param(t, "fnmode"),
        param(t, "iso_layout"),
        param(t, "swap_opt_cmd"),
        param(t, "swap_ctrl_cmd"),
        param(t, "swap_fn_leftctrl"),
        if t["module_loaded"].as_bool() == Some(true) { String::new() } else { tr!("  (hid_apple not loaded)") }
    );
    let inst = &t["installed"];
    o.push_str(&row(
        "hwdb:".into(),
        match inst["error"].as_str() {
            Some(e) => e.to_string(),
            None if inst["present"].as_bool() == Some(true) => trn!(
                "{path} ({n} key remapped)",
                "{path} ({n} keys remapped)",
                inst["overrides"].as_u64().unwrap_or(0),
                path = inst["path"].as_str().unwrap_or(""),
                n = inst["overrides"]
            ),
            None => tr!("none (kernel mapping)"),
        },
    ));
    let pending = if t["pending"].as_bool() == Some(true) {
        tr!(" - NOT applied yet: akmctl keymap apply")
    } else {
        String::new()
    };
    o.push_str(&row(
        tr!("Profile:"),
        tr!(
            "{profile} (preset {preset}){pending}",
            profile = t["profile"].as_str().unwrap_or("?"),
            preset = t["preset"]
                .as_str()
                .map_or_else(|| tr!("none"), str::to_string),
            pending = pending
        ),
    ));
    if let Err(e) = kde {
        o.push_str(&row(
            "KDE:".into(),
            tr!("not reachable ({e}); actions not shown", e = e),
        ));
    }
    o.push('\n');
    let mut missing = 0;
    for r in t["rows"].as_array().into_iter().flatten() {
        let key = r["key"].as_str().unwrap_or("?");
        let legend = akm_core::i18n::gettext(r["legend"].as_str().unwrap_or(""));
        let remap = if r["remapped"].as_bool() == Some(true) {
            format!("  [hwdb: {}]", r["base"].as_str().unwrap_or("?"))
        } else {
            String::new()
        };
        let _ = writeln!(o, "{key:<6} {legend}{remap}");
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
            let _ = writeln!(
                o,
                "   {mark}{label:<7} {code:<22} {sym:<22} {}",
                action.unwrap_or_default()
            );
        }
        if let Some(n) = r["note"].as_str() {
            let _ = writeln!(o, "          {}", tr!("note: {n}", n = n));
        }
    }
    if check {
        let _ = writeln!(o,
            "\n{}",
            trn!(
                "{missing} legend key without a KDE shortcut. Nothing was pressed or read: press each key and tick it in docs/KEYS.md.",
                "{missing} legend keys without a KDE shortcut. Nothing was pressed or read: press each key and tick it in docs/KEYS.md.",
                missing,
                missing = missing
            )
        );
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::keymap::{HidState, Keymap};
    use akm_core::keytable::table_json;

    fn table() -> Value {
        let p = HidState {
            fnmode: Some(1),
            ..HidState::KERNEL_DEFAULT
        };
        table_json(Some(0x0256), &p, &Ok(vec![]), &Keymap::default(), false)
    }

    #[allow(clippy::unnecessary_wraps)] // builds the arms of `measured`, which returns Option
    fn act(c: &str, a: &str) -> Option<Action> {
        Some(Action {
            component: c.into(),
            action: a.into(),
            component_name: c.into(),
            action_name: a.into(),
        })
    }

    #[allow(clippy::unnecessary_wraps)] // stands for kde::action_for, which can fail
    fn measured(q: i32) -> Result<Option<Action>, String> {
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
        assert!(
            s.contains("3 legend keys without a KDE shortcut"),
            "F4, F6, Eject: {s}"
        );
        let s = to_text(&t, &Err("no session bus".into()), false);
        assert!(s.contains("KDE:        not reachable") && s.contains("KEY_VOLUMEUP"));
    }
}
