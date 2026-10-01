//! Tab "Touches" (#247): effective table of the special keys and editor of
//! the manual mapping, through the daemon interface
//! `com.agenceapi.AppleKbMonitor1.Keymap` (KeyTable, Keymap, SetKey,
//! SetPreset, Apply, Reset) and KGlobalAccel for the KDE action of each key.
//!
//! No I/O on the UI thread: every D-Bus call runs on a worker thread, waited
//! for at most [`READ_TIMEOUT`] (reads) or [`APPLY_TIMEOUT`] (polkit dialog);
//! one job at a time, so repeated clicks never pile up threads. The UI only
//! reads [`TabState`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui;
use serde_json::Value;
use zbus::blocking::Connection;

const IFACE: &str = "com.agenceapi.AppleKbMonitor1.Keymap";
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
pub const APPLY_TIMEOUT: Duration = Duration::from_secs(150);

#[derive(Default, Clone)]
struct TabState {
    table: Option<Value>,
    keymap: Option<Value>,
    /// Qt key → KDE action label (None = nothing bound); empty map + flag
    /// when KDE is not reachable.
    kde: BTreeMap<u64, Option<String>>,
    kde_ok: bool,
    status: Option<(bool, String)>,
    busy: bool,
    requested: bool,
    edit_key: String,
    edit_code: String,
    edit_preset: String,
}

struct Tab {
    state: Mutex<TabState>,
    inflight: AtomicBool,
}

fn tab() -> &'static Arc<Tab> {
    static T: OnceLock<Arc<Tab>> = OnceLock::new();
    T.get_or_init(|| {
        Arc::new(Tab {
            state: Mutex::new(TabState { edit_key: "F6".into(), edit_preset: "none".into(), ..Default::default() }),
            inflight: AtomicBool::new(false),
        })
    })
}

fn lock(t: &Tab) -> std::sync::MutexGuard<'_, TabState> {
    t.state.lock().unwrap_or_else(|e| e.into_inner())
}

fn call(conn: &Connection, method: &str, body: &(impl zbus::export::serde::Serialize + zbus::zvariant::DynamicType)) -> Result<String, String> {
    let r = conn
        .call_method(
            Some(apple_kb_monitord::service::BUS_NAME),
            apple_kb_monitord::service::OBJECT_PATH,
            Some(IFACE),
            method,
            body,
        )
        .map_err(|e| match e {
            zbus::Error::MethodError(_, Some(m), _) => m,
            e => e.to_string(),
        })?;
    r.body().deserialize::<String>().map_err(|e| e.to_string())
}

fn kde_action(conn: &Connection, qt: u64) -> Result<Option<String>, String> {
    let r = conn
        .call_method(Some("org.kde.kglobalaccel"), "/kglobalaccel", Some("org.kde.KGlobalAccel"), "action", &(qt as i32))
        .map_err(|e| e.to_string())?;
    let v: Vec<String> = r.body().deserialize().map_err(|e| e.to_string())?;
    Ok(match v.as_slice() {
        [_, _, comp, act, ..] => Some(format!("{act} ({comp})")),
        _ => None,
    })
}

/// Everything the tab shows, loaded in one go (worker thread).
/// Table, keymap, KDE action per Qt key, KDE reachable.
type Loaded = (Value, Value, BTreeMap<u64, Option<String>>, bool);

fn load(conn: &Connection) -> Result<Loaded, String> {
    let table: Value = serde_json::from_str(&call(conn, "KeyTable", &(false,))?).map_err(|e| e.to_string())?;
    let keymap: Value = serde_json::from_str(&call(conn, "Keymap", &())?).map_err(|e| e.to_string())?;
    let mut kde = BTreeMap::new();
    let mut kde_ok = true;
    'rows: for r in table["rows"].as_array().into_iter().flatten() {
        for side in ["plain", "fn"] {
            if let Some(q) = r[side]["qt_key"].as_u64() {
                if kde.contains_key(&q) {
                    continue;
                }
                match kde_action(conn, q) {
                    Ok(a) => {
                        kde.insert(q, a);
                    }
                    Err(_) => {
                        kde_ok = false;
                        break 'rows;
                    }
                }
            }
        }
    }
    Ok((table, keymap, kde, kde_ok))
}

/// Runs `job` then reloads, off the UI thread, bounded by `timeout`.
fn spawn(ctx: egui::Context, timeout: Duration, job: impl FnOnce(&Connection) -> Result<Option<String>, String> + Send + 'static) {
    let t = tab().clone();
    if t.inflight.swap(true, Ordering::AcqRel) {
        return;
    }
    lock(&t).busy = true;
    let t2 = t.clone();
    let spawned = std::thread::Builder::new().name("keys-tab".into()).spawn(move || {
        let (tx, rx) = mpsc::channel();
        let inner = std::thread::Builder::new().name("keys-tab-dbus".into()).spawn(move || {
            let r = Connection::session().map_err(|e| e.to_string()).and_then(|c| {
                let msg = job(&c)?;
                load(&c).map(|l| (msg, l))
            });
            let _ = tx.send(r);
        });
        let outcome = match inner {
            Err(e) => Err(e.to_string()),
            Ok(_) => rx.recv_timeout(timeout).unwrap_or_else(|_| Err(format!("daemon did not answer within {} s", timeout.as_secs()))),
        };
        {
            let mut s = lock(&t2);
            match outcome {
                Ok((msg, (table, keymap, kde, kde_ok))) => {
                    s.table = Some(table);
                    s.keymap = Some(keymap);
                    s.kde = kde;
                    s.kde_ok = kde_ok;
                    if let Some(m) = msg {
                        s.status = Some((true, m));
                    }
                }
                Err(e) => s.status = Some((false, e)),
            }
            s.busy = false;
        }
        t2.inflight.store(false, Ordering::Release);
        ctx.request_repaint();
    });
    if spawned.is_err() {
        lock(&t).busy = false;
        t.inflight.store(false, Ordering::Release);
    }
}

fn kde_label(s: &TabState, side: &Value) -> String {
    let code = side["code"].as_str().unwrap_or("?");
    match side["qt_key"].as_u64() {
        _ if !s.kde_ok => String::new(),
        Some(q) => match s.kde.get(&q).cloned().flatten() {
            Some(a) => a,
            None if code.starts_with("KEY_F") => "→ application".into(),
            None => "aucune action KDE".into(),
        },
        None => "→ application".into(),
    }
}

/// Draws the tab. Never blocks: loads once, then on demand.
pub fn show(ui: &mut egui::Ui) {
    let t = tab().clone();
    let ctx = ui.ctx().clone();
    if !lock(&t).requested {
        lock(&t).requested = true;
        spawn(ctx.clone(), READ_TIMEOUT, |_| Ok(None));
    }
    let mut s = lock(&t).clone();

    ui.horizontal(|ui| {
        ui.heading("Touches spéciales");
        if ui.add_enabled(!s.busy, egui::Button::new("Actualiser")).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, |_| Ok(None));
        }
        if s.busy {
            ui.spinner();
        }
    });

    let Some(table) = s.table.clone() else {
        match &s.status {
            Some((_, e)) => ui.label(format!("Démon injoignable : {e}")),
            None => ui.label("Chargement…"),
        };
        return;
    };
    let p = &table["params"];
    let pv = |n: &str| p[n].as_i64().map_or("?".into(), |v| v.to_string());
    ui.label(format!(
        "Clavier 05ac:{}{} · hid_apple fnmode={} swap_opt_cmd={} iso_layout={} · profil {} (préset {}){}",
        table["pid"].as_str().unwrap_or("?"),
        if table["connected"].as_bool() == Some(true) { "" } else { " (non connecté)" },
        pv("fnmode"),
        pv("swap_opt_cmd"),
        pv("iso_layout"),
        table["profile"].as_str().unwrap_or("?"),
        table["preset"].as_str().unwrap_or("aucun"),
        if table["pending"].as_bool() == Some(true) { " · NON appliqué" } else { "" },
    ));
    if !s.kde_ok {
        ui.label("KDE (KGlobalAccel) injoignable : actions non affichées.");
    }
    ui.add_space(6.0);

    egui::ScrollArea::vertical().max_height(ui.available_height() * 0.6).show(ui, |ui| {
        egui::Grid::new("keys-table").striped(true).num_columns(4).show(ui, |ui| {
            for h in ["Touche", "Sans Fn", "Avec Fn", "Note"] {
                ui.strong(h);
            }
            ui.end_row();
            for r in table["rows"].as_array().into_iter().flatten() {
                let remap = if r["remapped"].as_bool() == Some(true) { " *" } else { "" };
                ui.label(format!("{}{remap}  {}", r["key"].as_str().unwrap_or("?"), r["legend"].as_str().unwrap_or("")));
                for side in [&r["plain"], &r["fn"]] {
                    ui.label(format!("{}\n{}", side["code"].as_str().unwrap_or("?"), kde_label(&s, side)));
                }
                ui.add(egui::Label::new(r["note"].as_str().unwrap_or("")).wrap());
                ui.end_row();
            }
        });
    });

    ui.separator();
    ui.label("Mapping manuel (udev hwdb, sans keyd ; rien ne change avant « Appliquer », mot de passe administrateur) :");
    let ids: Vec<String> = s.keymap.as_ref().and_then(|k| k["keys"].as_array().cloned()).unwrap_or_default().iter().filter_map(|k| k["id"].as_str().map(String::from)).collect();
    let mut changed = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("keys-tab-key").selected_text(s.edit_key.clone()).show_ui(ui, |ui| {
            for id in &ids {
                changed |= ui.selectable_value(&mut s.edit_key, id.clone(), id).changed();
            }
        });
        ui.label("→");
        changed |= ui.add(egui::TextEdit::singleline(&mut s.edit_code).hint_text("KEY_F13").desired_width(160.0)).changed();
        let (key, code) = (s.edit_key.clone(), s.edit_code.trim().to_string());
        if ui.add_enabled(!s.busy && !code.is_empty(), egui::Button::new("Remapper")).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, move |c| call(c, "SetKey", &("", key.as_str(), code.as_str())).map(|_| Some("Enregistré (pas encore appliqué)".into())));
        }
        let key = s.edit_key.clone();
        if ui.add_enabled(!s.busy, egui::Button::new("Rétablir cette touche")).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, move |c| call(c, "SetKey", &("", key.as_str(), "")).map(|_| Some("Enregistré (pas encore appliqué)".into())));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Préset :");
        let presets: Vec<(String, String)> = std::iter::once(("none".to_string(), "Aucun (paramètres actuels conservés)".to_string()))
            .chain(s
            .keymap
            .as_ref()
            .and_then(|k| k["presets"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|p| (p["name"].as_str().unwrap_or("").to_string(), p["title"].as_str().unwrap_or("").to_string())))
            .collect();
        let cur = presets.iter().find(|(n, _)| *n == s.edit_preset).map_or(s.edit_preset.clone(), |(_, t)| t.clone());
        egui::ComboBox::from_id_salt("keys-tab-preset").selected_text(cur).show_ui(ui, |ui| {
            for (n, title) in &presets {
                changed |= ui.selectable_value(&mut s.edit_preset, n.clone(), title).changed();
            }
        });
        let pr = s.edit_preset.clone();
        if ui.add_enabled(!s.busy, egui::Button::new("Choisir")).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, move |c| call(c, "SetPreset", &("", pr.as_str())).map(|_| Some("Préset enregistré (pas encore appliqué)".into())));
        }
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(!s.busy, egui::Button::new("Appliquer")).clicked() {
            spawn(ctx.clone(), APPLY_TIMEOUT, |c| call(c, "Apply", &()).map(Some));
        }
        if ui.add_enabled(!s.busy, egui::Button::new("Revenir au mapping du noyau")).clicked() {
            spawn(ctx.clone(), APPLY_TIMEOUT, |c| call(c, "Reset", &()).map(Some));
        }
        ui.label("hid_apple s'applique à tous les claviers Apple.");
    });
    if let Some((ok, m)) = &s.status {
        ui.label(if *ok { m.clone() } else { format!("Erreur : {m}") });
    }
    if changed {
        let mut g = lock(&t);
        g.edit_key = s.edit_key;
        g.edit_code = s.edit_code;
        g.edit_preset = s.edit_preset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_without_kde_never_claim_an_action() {
        let s = TabState { kde_ok: false, ..Default::default() };
        let side = serde_json::json!({"code": "KEY_SCALE", "qt_key": 16777390u64});
        assert_eq!(kde_label(&s, &side), "");
        let mut s = TabState { kde_ok: true, ..Default::default() };
        s.kde.insert(16777390, Some("ExposeAll (KWin)".into()));
        assert_eq!(kde_label(&s, &side), "ExposeAll (KWin)");
        let f = serde_json::json!({"code": "KEY_F4", "qt_key": 16777267u64});
        assert_eq!(kde_label(&s, &f), "→ application");
        let e = serde_json::json!({"code": "KEY_EJECTCD", "qt_key": 16777401u64});
        assert_eq!(kde_label(&s, &e), "aucune action KDE");
    }

    /// The UI thread never waits: a job that never ends leaves the tab busy
    /// and refuses a second job instead of stacking threads.
    #[test]
    fn one_job_at_a_time() {
        let t = tab().clone();
        t.inflight.store(true, Ordering::Release);
        let before = lock(&t).busy;
        spawn(egui::Context::default(), READ_TIMEOUT, |_| unreachable!("second job"));
        assert_eq!(lock(&t).busy, before);
        t.inflight.store(false, Ordering::Release);
    }
}
