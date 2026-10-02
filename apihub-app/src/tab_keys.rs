//! KEYS tab (#247): the Fn switch, the effective table of the special keys and editor of
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

use crate::actions::FnMode;
use crate::i18n::tr;
use crate::theme::{self, Theme};

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
            state: Mutex::new(TabState {
                edit_key: "F6".into(),
                edit_preset: "none".into(),
                ..Default::default()
            }),
            inflight: AtomicBool::new(false),
        })
    })
}

fn lock(t: &Tab) -> std::sync::MutexGuard<'_, TabState> {
    t.state.lock().unwrap_or_else(|e| e.into_inner())
}

fn call(
    conn: &Connection,
    method: &str,
    body: &(impl zbus::export::serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<String, String> {
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
        .call_method(
            Some("org.kde.kglobalaccel"),
            "/kglobalaccel",
            Some("org.kde.KGlobalAccel"),
            "action",
            &(qt as i32),
        )
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
    let table: Value =
        serde_json::from_str(&call(conn, "KeyTable", &(false,))?).map_err(|e| e.to_string())?;
    let keymap: Value =
        serde_json::from_str(&call(conn, "Keymap", &())?).map_err(|e| e.to_string())?;
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
fn spawn(
    ctx: egui::Context,
    timeout: Duration,
    job: impl FnOnce(&Connection) -> Result<Option<String>, String> + Send + 'static,
) {
    let t = tab().clone();
    if t.inflight.swap(true, Ordering::AcqRel) {
        return;
    }
    lock(&t).busy = true;
    let t2 = t.clone();
    let spawned = std::thread::Builder::new()
        .name("keys-tab".into())
        .spawn(move || {
            let (tx, rx) = mpsc::channel();
            let inner = std::thread::Builder::new()
                .name("keys-tab-dbus".into())
                .spawn(move || {
                    let r = Connection::session()
                        .map_err(|e| e.to_string())
                        .and_then(|c| {
                            let msg = job(&c)?;
                            load(&c).map(|l| (msg, l))
                        });
                    let _ = tx.send(r);
                });
            let outcome = match inner {
                Err(e) => Err(e.to_string()),
                Ok(_) => rx.recv_timeout(timeout).unwrap_or_else(|_| {
                    Err(crate::i18n::trf(
                        "daemon did not answer within {} s",
                        &[&timeout.as_secs()],
                    ))
                }),
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
            None if code.starts_with("KEY_F") => tr("→ application").into(),
            None => tr("no KDE action").into(),
        },
        None => tr("→ application").into(),
    }
}

/// Reload the table and the keymap (Refresh button, F5).
pub fn reload(ctx: &egui::Context) {
    spawn(ctx.clone(), READ_TIMEOUT, |_| Ok(None));
}

/// Draws the tab. Never blocks: loads once, then on demand.
pub fn show(ui: &mut egui::Ui, th: &Theme, fnmode: &FnMode) {
    let t = tab().clone();
    let ctx = ui.ctx().clone();
    if !lock(&t).requested {
        lock(&t).requested = true;
        reload(&ctx);
    }
    let mut s = lock(&t).clone();
    let mut changed = false;
    theme::scroll_body(ui, crate::shell::Tab::Keys.label(), |ui| {
        fn_panel(ui, th, fnmode);
        ui.add_space(theme::GAP);
        th.panel(ui, tr("Special keys"), 0.0, |ui| table(ui, th, &s));
        ui.add_space(theme::GAP);
        th.panel(ui, tr("Manual mapping"), 0.0, |ui| {
            changed = editor(ui, th, &mut s);
        });
    });
    if changed {
        let mut g = lock(&t);
        g.edit_key = s.edit_key;
        g.edit_code = s.edit_code;
        g.edit_preset = s.edit_preset;
    }
}

/// Which side of the Fn switch is on: `Some(true)` media keys first,
/// `Some(false)` F1–F12 first, `None` when the mode is neither (Fn disabled,
/// F-keys disabled, not read): the switch then gives way to a plain toggle.
pub fn media_first(mode: Option<i32>) -> Option<bool> {
    match mode {
        Some(1 | 3) => Some(true),
        Some(crate::fn_toggle::FKEYS_FIRST) => Some(false),
        _ => None,
    }
}

/// The Fn switch: the two modes side by side, the current one in inverse
/// video; choosing the other one toggles through the daemon (polkit).
fn fn_panel(ui: &mut egui::Ui, th: &Theme, fnmode: &FnMode) {
    th.panel(ui, tr("Function keys"), 0.0, |ui| {
        let ctx = ui.ctx().clone();
        let mode = fnmode.mode();
        ui.horizontal_wrapped(|ui| match media_first(mode) {
            Some(media) => {
                for (side, text) in [
                    (
                        true,
                        crate::fn_toggle::mode_text(crate::fn_toggle::MEDIA_FIRST),
                    ),
                    (
                        false,
                        crate::fn_toggle::mode_text(crate::fn_toggle::FKEYS_FIRST),
                    ),
                ] {
                    let on = side == media;
                    let r =
                        theme::choice(ui, on, &text).on_hover_text(tr("Toggle the function keys"));
                    if r.clicked() && !on {
                        let ctx = ctx.clone();
                        fnmode.toggle(move || ctx.request_repaint());
                    }
                }
            }
            None => crate::tab_stat::fn_button(ui, fnmode),
        });
        crate::shell::outcome(ui, th, &fnmode.job.outcome());
        theme::text(
            ui,
            tr("hid_apple applies to all Apple keyboards."),
            theme::SMALL,
            theme::GREEN_MID,
        );
    });
}

/// Width of a keycap: its text and some padding, never narrower than a
/// one-letter key.
const KEYCAP_MIN: f32 = 34.0;
/// Room kept for the keycap in the first column of the wide table.
const KEYCAP_COLUMN: f32 = 64.0;

/// Paints a keycap whose left edge is at `at`, centred on its `y`.
fn paint_keycap(p: &egui::Painter, at: egui::Pos2, id: &str) -> egui::Rect {
    let g = p.layout_no_wrap(id.to_string(), theme::font(theme::BODY), theme::PHOSPHOR);
    let size = egui::Vec2::new((g.size().x + 14.0).max(KEYCAP_MIN), theme::ROW);
    let rect = egui::Rect::from_min_size(egui::Pos2::new(at.x, at.y - size.y / 2.0), size);
    p.rect(
        rect.shrink(0.5),
        2.0,
        theme::BG,
        egui::Stroke::new(1.0, theme::GREEN_MID),
        egui::StrokeKind::Inside,
    );
    p.hline(
        rect.x_range().shrink(2.0),
        rect.bottom() - 2.0,
        egui::Stroke::new(1.0, theme::GREEN_MID),
    );
    p.galley(rect.center() - g.size() / 2.0, g, theme::PHOSPHOR);
    rect
}

/// A key drawn as a keycap.
fn keycap(ui: &mut egui::Ui, id: &str) {
    let chars = id.chars().count() as f32;
    let size = egui::Vec2::new(
        (chars * theme::ADVANCE * theme::BODY + 14.0).max(KEYCAP_MIN),
        theme::ROW,
    );
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    paint_keycap(ui.painter(), rect.left_center(), id);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, id));
}

/// Widths of the four columns of the wide table (key, without Fn, with Fn,
/// note) and the space between two columns.
const COLUMN_GAP: f32 = 16.0;
pub fn columns(avail: f32) -> [f32; 4] {
    let w = (avail - 3.0 * COLUMN_GAP).max(4.0);
    [0.22, 0.28, 0.24, 0.26].map(|share| (w * share).floor())
}

/// One row of the wide table: every cell wrapped in its column, the row as
/// tall as its tallest cell. `key`: the keycap drawn before the first cell.
fn table_row(
    ui: &mut egui::Ui,
    cols: [f32; 4],
    key: Option<&str>,
    cells: [&str; 4],
    ink: egui::Color32,
) {
    let lead = if key.is_some() { KEYCAP_COLUMN } else { 0.0 };
    let p = ui.painter();
    let galleys: Vec<_> = cells
        .iter()
        .zip(cols)
        .enumerate()
        .map(|(i, (text, w))| {
            let w = if i == 0 { w - lead } else { w };
            // The legend of the key is the primary text of the row.
            let color = if i == 3 { theme::GREEN_MID } else { ink };
            p.layout(
                theme::glyphs(text).into_owned(),
                theme::font(theme::BODY),
                color,
                w.max(20.0),
            )
        })
        .collect();
    let h = galleys
        .iter()
        .map(|g| g.size().y)
        .fold(theme::ROW, f32::max)
        + 6.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::Vec2::new(ui.available_width(), h),
        egui::Sense::hover(),
    );
    let p = ui.painter();
    let mut x = rect.left();
    for (i, (g, w)) in galleys.into_iter().zip(cols).enumerate() {
        let at = if i == 0 { x + lead } else { x };
        p.galley(egui::Pos2::new(at, rect.top() + 3.0), g, ink);
        x += w + COLUMN_GAP;
    }
    if let Some(id) = key {
        paint_keycap(
            p,
            egui::Pos2::new(rect.left(), rect.top() + 3.0 + theme::ROW / 2.0 - 2.0),
            id,
        );
    }
    let said = key.into_iter().chain(cells).collect::<Vec<_>>().join(", ");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
}

/// What one side (without / with Fn) of a key does: its code, then the KDE
/// action when there is one.
fn side_text(s: &TabState, side: &Value) -> String {
    let code = side["code"].as_str().unwrap_or("?");
    let kde = kde_label(s, side);
    if kde.is_empty() {
        code.to_string()
    } else {
        format!("{code} \u{b7} {kde}")
    }
}

fn table(ui: &mut egui::Ui, th: &Theme, s: &TabState) {
    let ctx = ui.ctx().clone();
    ui.horizontal_wrapped(|ui| {
        if theme::action(ui, tr("Refresh"), !s.busy).clicked() {
            reload(&ctx);
        }
        if s.busy {
            theme::text(ui, tr("Loading…"), theme::BODY, theme::AMBER);
        }
    });
    let Some(table) = s.table.as_ref() else {
        match &s.status {
            Some((_, e)) => th.alert(
                ui,
                crate::view::Level::Bad,
                &crate::i18n::trf("Daemon unreachable: {}", &[e]),
            ),
            None => {
                theme::text(ui, tr("Loading…"), theme::BODY, theme::GREEN_MID);
            }
        };
        return;
    };
    let p = &table["params"];
    let pv = |n: &str| p[n].as_i64().map_or("?".into(), |v| v.to_string());
    let not_connected = if table["connected"].as_bool() == Some(true) {
        String::new()
    } else {
        format!(" {}", tr("(not connected)"))
    };
    let pending = if table["pending"].as_bool() == Some(true) {
        format!(" · {}", tr("NOT applied"))
    } else {
        String::new()
    };
    theme::text(
        ui,
        &crate::i18n::trf(
            "Keyboard 05ac:{}{} · hid_apple fnmode={} swap_opt_cmd={} iso_layout={} · profile {} (preset {}){}",
            &[
                &table["pid"].as_str().unwrap_or("?"),
                &not_connected,
                &pv("fnmode"),
                &pv("swap_opt_cmd"),
                &pv("iso_layout"),
                &table["profile"].as_str().unwrap_or("?"),
                &table["preset"].as_str().unwrap_or(tr("none")),
                &pending,
            ],
        ),
        theme::SMALL,
        theme::GREEN_MID,
    );
    if !s.kde_ok {
        th.alert(
            ui,
            crate::view::Level::Warn,
            tr("KDE (KGlobalAccel) unreachable: actions not shown."),
        );
    }
    theme::rule(ui);
    let rows: Vec<&Value> = table["rows"].as_array().into_iter().flatten().collect();
    if ui.available_width() >= theme::NARROW {
        let cols = columns(ui.available_width());
        let head = [tr("Key"), tr("Without Fn"), tr("With Fn"), tr("Note")].map(theme::caps);
        table_row(
            ui,
            cols,
            None,
            [&head[0], &head[1], &head[2], &head[3]],
            theme::GREEN_MID,
        );
        for r in rows {
            let legend = match r["remapped"].as_bool() {
                Some(true) => format!("{} *", r["legend"].as_str().unwrap_or("")),
                _ => r["legend"].as_str().unwrap_or("").to_string(),
            };
            table_row(
                ui,
                cols,
                Some(r["key"].as_str().unwrap_or("?")),
                [
                    &legend,
                    &side_text(s, &r["plain"]),
                    &side_text(s, &r["fn"]),
                    r["note"].as_str().unwrap_or(""),
                ],
                theme::PHOSPHOR,
            );
        }
    } else {
        // Narrow window: one block per key, nothing truncated.
        for (i, r) in rows.into_iter().enumerate() {
            if i > 0 {
                theme::rule(ui);
            }
            ui.horizontal_wrapped(|ui| key_title(ui, r));
            th.kv(
                ui,
                tr("Without Fn"),
                &side_text(s, &r["plain"]),
                crate::view::Level::Unknown,
            );
            th.kv(
                ui,
                tr("With Fn"),
                &side_text(s, &r["fn"]),
                crate::view::Level::Unknown,
            );
            let note = r["note"].as_str().unwrap_or("");
            if !note.is_empty() {
                theme::text(ui, note, theme::SMALL, theme::GREEN_MID);
            }
        }
    }
}

/// Keycap, legend and the mark of a remapped key.
fn key_title(ui: &mut egui::Ui, r: &Value) {
    keycap(ui, r["key"].as_str().unwrap_or("?"));
    theme::text(
        ui,
        r["legend"].as_str().unwrap_or(""),
        theme::BODY,
        theme::PHOSPHOR,
    );
    if r["remapped"].as_bool() == Some(true) {
        theme::text(ui, "*", theme::BODY, theme::AMBER);
    }
}

/// Editor of the manual mapping. Returns true when a field changed.
fn editor(ui: &mut egui::Ui, th: &Theme, s: &mut TabState) -> bool {
    let ctx = ui.ctx().clone();
    theme::text(
        ui,
        tr("Manual mapping (udev hwdb, without keyd; nothing changes until \"Apply\", administrator password):"),
        theme::BODY,
        theme::GREEN_MID,
    );
    let ids: Vec<String> = s
        .keymap
        .as_ref()
        .and_then(|k| k["keys"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|k| k["id"].as_str().map(String::from))
        .collect();
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("keys-tab-key")
            .selected_text(s.edit_key.clone())
            .show_ui(ui, |ui| {
                for id in &ids {
                    changed |= ui
                        .selectable_value(&mut s.edit_key, id.clone(), id)
                        .changed();
                }
            });
        theme::text(ui, ">", theme::BODY, theme::GREEN_MID);
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut s.edit_code)
                    .hint_text("KEY_F13")
                    .desired_width(150.0)
                    .margin(egui::Margin::symmetric(6, 5)),
            )
            .changed();
        let (key, code) = (s.edit_key.clone(), s.edit_code.trim().to_string());
        if theme::action(ui, tr("Remap"), !s.busy && !code.is_empty()).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, move |c| {
                call(c, "SetKey", &("", key.as_str(), code.as_str()))
                    .map(|_| Some(tr("Saved (not applied yet)").into()))
            });
        }
        let key = s.edit_key.clone();
        if theme::action(ui, tr("Restore this key"), !s.busy).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, move |c| {
                call(c, "SetKey", &("", key.as_str(), ""))
                    .map(|_| Some(tr("Saved (not applied yet)").into()))
            });
        }
    });
    ui.horizontal_wrapped(|ui| {
        theme::text(ui, tr("Preset:"), theme::BODY, theme::GREEN_MID);
        let presets: Vec<(String, String)> = std::iter::once((
            "none".to_string(),
            tr("None (current settings kept)").to_string(),
        ))
        .chain(
            s.keymap
                .as_ref()
                .and_then(|k| k["presets"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .map(|p| {
                    (
                        p["name"].as_str().unwrap_or("").to_string(),
                        theme::glyphs(p["title"].as_str().unwrap_or("")).into_owned(),
                    )
                }),
        )
        .collect();
        let cur = presets
            .iter()
            .find(|(n, _)| *n == s.edit_preset)
            .map_or(s.edit_preset.clone(), |(_, t)| t.clone());
        egui::ComboBox::from_id_salt("keys-tab-preset")
            .selected_text(cur)
            .show_ui(ui, |ui| {
                for (n, title) in &presets {
                    changed |= ui
                        .selectable_value(&mut s.edit_preset, n.clone(), title)
                        .changed();
                }
            });
        let pr = s.edit_preset.clone();
        if theme::action(ui, tr("Choose"), !s.busy).clicked() {
            spawn(ctx.clone(), READ_TIMEOUT, move |c| {
                call(c, "SetPreset", &("", pr.as_str()))
                    .map(|_| Some(tr("Preset saved (not applied yet)").into()))
            });
        }
    });
    ui.horizontal_wrapped(|ui| {
        if theme::action(ui, tr("Apply"), !s.busy).clicked() {
            spawn(ctx.clone(), APPLY_TIMEOUT, |c| {
                call(c, "Apply", &()).map(Some)
            });
        }
        if theme::action(ui, tr("Back to the kernel mapping"), !s.busy).clicked() {
            spawn(ctx.clone(), APPLY_TIMEOUT, |c| {
                call(c, "Reset", &()).map(Some)
            });
        }
    });
    let shown = s.status.clone().map(|(ok, m)| {
        if ok {
            (ok, m)
        } else {
            (ok, crate::i18n::trf("Error: {}", &[&m]))
        }
    });
    // An unreachable daemon is already said above the table.
    if s.table.is_some() {
        crate::shell::outcome(ui, th, &shown);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_columns_fill_the_width_without_overflow() {
        for avail in [theme::NARROW, 846.0, 1150.0, 10.0] {
            let c = columns(avail);
            let total: f32 = c.iter().sum::<f32>() + 3.0 * COLUMN_GAP;
            assert!(
                total <= avail.max(3.0 * COLUMN_GAP + 4.0),
                "{avail}: {total}"
            );
            assert!(c.iter().all(|w| *w >= 0.0));
        }
        // The first column keeps room for the keycap and a legend.
        assert!(columns(theme::NARROW)[0] > KEYCAP_COLUMN + 8.0 * theme::ADVANCE * theme::BODY);
    }

    #[test]
    fn fn_switch_sides() {
        assert_eq!(media_first(Some(1)), Some(true));
        assert_eq!(
            media_first(Some(3)),
            Some(true),
            "auto behaves as media first"
        );
        assert_eq!(media_first(Some(2)), Some(false));
        for other in [None, Some(0), Some(4), Some(-1), Some(9)] {
            assert_eq!(media_first(other), None);
        }
        // The side that is off is the mode the toggle goes to.
        assert_eq!(
            crate::fn_toggle::next_mode(1),
            Some(crate::fn_toggle::FKEYS_FIRST)
        );
        assert_eq!(
            crate::fn_toggle::next_mode(2),
            Some(crate::fn_toggle::MEDIA_FIRST)
        );
    }

    #[test]
    fn a_side_shows_its_code_then_the_kde_action() {
        let mut s = TabState {
            kde_ok: true,
            ..Default::default()
        };
        s.kde.insert(16777390, Some("ExposeAll (KWin)".into()));
        let side = serde_json::json!({"code": "KEY_SCALE", "qt_key": 16777390u64});
        assert_eq!(side_text(&s, &side), "KEY_SCALE \u{b7} ExposeAll (KWin)");
        let s = TabState {
            kde_ok: false,
            ..Default::default()
        };
        assert_eq!(side_text(&s, &side), "KEY_SCALE");
    }

    #[test]
    fn labels_without_kde_never_claim_an_action() {
        let s = TabState {
            kde_ok: false,
            ..Default::default()
        };
        let side = serde_json::json!({"code": "KEY_SCALE", "qt_key": 16777390u64});
        assert_eq!(kde_label(&s, &side), "");
        let mut s = TabState {
            kde_ok: true,
            ..Default::default()
        };
        s.kde.insert(16777390, Some("ExposeAll (KWin)".into()));
        assert_eq!(kde_label(&s, &side), "ExposeAll (KWin)");
        let f = serde_json::json!({"code": "KEY_F4", "qt_key": 16777267u64});
        assert_eq!(kde_label(&s, &f), "→ application");
        let e = serde_json::json!({"code": "KEY_EJECTCD", "qt_key": 16777401u64});
        assert_eq!(kde_label(&s, &e), "no KDE action");
    }

    /// The UI thread never waits: a job that never ends leaves the tab busy
    /// and refuses a second job instead of stacking threads.
    #[test]
    fn one_job_at_a_time() {
        let t = tab().clone();
        t.inflight.store(true, Ordering::Release);
        let before = lock(&t).busy;
        spawn(egui::Context::default(), READ_TIMEOUT, |_| {
            unreachable!("second job")
        });
        assert_eq!(lock(&t).busy, before);
        t.inflight.store(false, Ordering::Release);
    }
}
