//! KDE global shortcuts (`org.kde.kglobalaccel`, session bus), read-only
//! except [`apply_missing`] which only ADDS a key to an action and only when
//! that key is bound to nothing (#247).

use zbus::blocking::Connection;

const DEST: &str = "org.kde.kglobalaccel";
const PATH: &str = "/kglobalaccel";
const IFACE: &str = "org.kde.KGlobalAccel";

/// Qt::Key_LaunchD: what F4 (KEY_ALL_APPLICATIONS / XF86LaunchB) reaches KDE as.
pub const QT_LAUNCH_D: i32 = 0x0100_00af;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub component: String,
    pub action: String,
    pub component_name: String,
    pub action_name: String,
}

impl Action {
    pub fn label(&self) -> String {
        format!("{} ({}: {})", self.action_name, self.component_name, self.action)
    }
}

/// Global shortcut bound to a Qt key, `Ok(None)` = nothing bound,
/// `Err` = KGlobalAccel not reachable (not a KDE session).
pub fn action_for(conn: &Connection, qt_key: u32) -> Result<Option<Action>, String> {
    let reply = conn
        .call_method(Some(DEST), PATH, Some(IFACE), "action", &(qt_key as i32))
        .map_err(|e| format!("KGlobalAccel: {e}"))?;
    let v: Vec<String> = reply.body().deserialize().map_err(|e| format!("KGlobalAccel: {e}"))?;
    Ok(match v.as_slice() {
        [c, a, cn, an, ..] => Some(Action { component: c.clone(), action: a.clone(), component_name: cn.clone(), action_name: an.clone() }),
        _ => None,
    })
}

/// Shortcuts KDE is missing for the Apple legend, added by `kde-apply`.
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

fn shortcut_keys(conn: &Connection, b: &Binding) -> Result<Keys, String> {
    let id = vec![b.component.to_string(), b.action.to_string()];
    let reply = conn
        .call_method(Some(DEST), PATH, Some(IFACE), "shortcutKeys", &(id,))
        .map_err(|e| format!("shortcutKeys {}/{}: {e}", b.component, b.action))?;
    reply.body().deserialize::<Keys>().map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Key already bound (to this action or another one): left alone.
    AlreadyBound(Action),
    /// Would add / added the key to the action, keeping its other keys.
    Add { existing: usize },
    /// Undo: the key was ours and is removed / not present.
    Removed,
    NotPresent,
}

/// Plan (and with `write`, apply) the missing bindings, never replacing an
/// existing one. `undo` removes exactly the keys of [`BINDINGS`] from their
/// actions, nothing else.
pub fn apply_missing(conn: &Connection, write: bool, undo: bool) -> Result<Vec<(Binding, Step)>, String> {
    let mut out = Vec::new();
    for b in BINDINGS {
        let keys = shortcut_keys(conn, b)?;
        let ours = keys.iter().position(|(k,)| k.first() == Some(&b.qt_key) && k.iter().skip(1).all(|x| *x == 0));
        let step = if undo {
            match ours {
                Some(i) => {
                    if write {
                        let mut k = keys.clone();
                        k.remove(i);
                        set_keys(conn, b, k)?;
                    }
                    Step::Removed
                }
                None => Step::NotPresent,
            }
        } else {
            match action_for(conn, b.qt_key as u32)? {
                Some(a) => Step::AlreadyBound(a),
                None => {
                    if write {
                        let mut k = keys.clone();
                        k.push((vec![b.qt_key],));
                        set_keys(conn, b, k)?;
                    }
                    Step::Add { existing: keys.len() }
                }
            }
        };
        out.push((*b, step));
    }
    Ok(out)
}

fn set_keys(conn: &Connection, b: &Binding, keys: Keys) -> Result<(), String> {
    let id = vec![b.component.to_string(), b.action.to_string()];
    conn.call_method(Some(DEST), PATH, Some(IFACE), "setForeignShortcutKeys", &(id, keys))
        .map(|_| ())
        .map_err(|e| format!("setForeignShortcutKeys {}/{}: {e}", b.component, b.action))
}
