//! KDE global shortcuts (`org.kde.kglobalaccel`, session bus), read-only except [`apply_missing`]
//! which only ADDS a key to an action and only when that key is bound to nothing.

use zbus::blocking::Connection;

const DEST: &str = "org.kde.kglobalaccel";
const PATH: &str = "/kglobalaccel";
const IFACE: &str = "org.kde.KGlobalAccel";

pub const QT_LAUNCH_D: i32 = 0x0100_00af;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)] // the four fields of a kglobalaccel action id
pub struct Action {
    pub component: String,
    pub action: String,
    pub component_name: String,
    pub action_name: String,
}

impl Action {
    pub fn label(&self) -> String {
        format!(
            "{} ({}: {})",
            self.action_name, self.component_name, self.action
        )
    }
}

pub fn action_for(conn: &Connection, qt_key: i32) -> Result<Option<Action>, String> {
    let reply = conn
        .call_method(Some(DEST), PATH, Some(IFACE), "action", &qt_key)
        .map_err(|e| format!("KGlobalAccel: {e}"))?;
    let v: Vec<String> = reply
        .body()
        .deserialize()
        .map_err(|e| format!("KGlobalAccel: {e}"))?;
    Ok(match v.as_slice() {
        [c, a, cn, an, ..] => Some(Action {
            component: c.clone(),
            action: a.clone(),
            component_name: cn.clone(),
            action_name: an.clone(),
        }),
        _ => None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub key: &'static str,
    pub qt_key: i32,
    pub qt_name: &'static str,
    pub component: &'static str,
    pub action: &'static str,
    pub why: &'static str,
}

pub const BINDINGS: &[Binding] = &[Binding {
    key: "F4",
    qt_key: QT_LAUNCH_D,
    qt_name: "Launch (D)",
    component: "plasmashell",
    action: "activate application launcher",
    why: "F4 = Launchpad (KEY_ALL_APPLICATIONS) → application launcher",
}];

type Keys = Vec<(Vec<i32>,)>;

/// Full action id: `KGlobalAccel` answers `shortcutKeys` with nothing for a 2-entry id.
fn action_id(conn: &Connection, b: &Binding) -> Result<Vec<String>, String> {
    let reply = conn
        .call_method(
            Some(DEST),
            PATH,
            Some(IFACE),
            "allActionsForComponent",
            &(vec![b.component.to_string()],),
        )
        .map_err(|e| format!("allActionsForComponent {}: {e}", b.component))?;
    let all: Vec<Vec<String>> = reply.body().deserialize().map_err(|e| e.to_string())?;
    all.into_iter()
        .find(|id| id.len() >= 4 && id[0] == b.component && id[1] == b.action)
        .ok_or_else(|| {
            format!(
                "{}/{} not registered in KGlobalAccel",
                b.component, b.action
            )
        })
}

fn shortcut_keys(conn: &Connection, id: &[String]) -> Result<Keys, String> {
    let reply = conn
        .call_method(Some(DEST), PATH, Some(IFACE), "shortcutKeys", &(id,))
        .map_err(|e| format!("shortcutKeys {}/{}: {e}", id[0], id[1]))?;
    reply
        .body()
        .deserialize::<Keys>()
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Key already bound (to this action or another one): left alone.
    AlreadyBound(Action),
    /// Would add / added the key to the action, keeping its other keys.
    Add {
        existing: usize,
    },
    /// Undo: the key was ours and is removed / not present.
    Removed,
    NotPresent,
}

/// Plan (and with `write`, apply) the missing bindings, never replacing an existing one.
pub fn apply_missing(
    conn: &Connection,
    write: bool,
    undo: bool,
) -> Result<Vec<(Binding, Step)>, String> {
    let mut out = Vec::new();
    for b in BINDINGS {
        let id = action_id(conn, b)?;
        let keys = shortcut_keys(conn, &id)?;
        let ours = keys
            .iter()
            .position(|(k,)| k.first() == Some(&b.qt_key) && k.iter().skip(1).all(|x| *x == 0));
        let step = if undo {
            match ours {
                Some(i) => {
                    if write {
                        let mut k = keys.clone();
                        k.remove(i);
                        set_keys(conn, &id, k)?;
                    }
                    Step::Removed
                }
                None => Step::NotPresent,
            }
        } else if let Some(a) = action_for(conn, b.qt_key)? {
            Step::AlreadyBound(a)
        } else {
            if write {
                let mut k = keys.clone();
                k.push((vec![b.qt_key],));
                set_keys(conn, &id, k)?;
            }
            Step::Add {
                existing: keys.len(),
            }
        };
        out.push((*b, step));
    }
    Ok(out)
}

fn set_keys(conn: &Connection, id: &[String], keys: Keys) -> Result<(), String> {
    conn.call_method(
        Some(DEST),
        PATH,
        Some(IFACE),
        "setForeignShortcutKeys",
        &(id, keys),
    )
    .map(|_| ())
    .map_err(|e| format!("setForeignShortcutKeys {}/{}: {e}", id[0], id[1]))
}
