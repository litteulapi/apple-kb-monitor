//! DATA tab: what explains the figures of STAT. Composition: the history
//! chart across the tab, then the batteries (with the voltage on the scale
//! of the keyboard's own thresholds) beside the device sheet, where the
//! keyboard is renamed, and the firmware.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align2, Pos2, Sense, Stroke, Ui, Vec2};

use akm_core::report::{KbBattery, KbReport};
use akm_core::Snapshot;

use crate::history_view::Loader;
use crate::i18n::{tr, trf};
use crate::theme::{self, Theme};
use crate::view::{self, Level, Range};
use crate::{history_chart, rename, shell};

/// State of the name field (#141).
pub struct Rename {
    buf: String,
    loaded: Option<String>,
    status: rename::Status,
    /// Set by STAT's Rename button: focus the field at the next frame.
    pub focus: bool,
}

impl Rename {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            loaded: None,
            status: Arc::new(Mutex::new(None)),
            focus: false,
        }
    }
}

/// The command that writes the name INTO the keyboard; the window only
/// shows it (it never writes into the keyboard).
pub fn device_name_command(name: &str) -> String {
    let name = if name.trim().is_empty() {
        tr("NAME")
    } else {
        name.trim()
    };
    format!("akmctl rename --device-name {}", shell_quote(name))
}

/// One shell word that a POSIX shell never expands (#276): single quotes,
/// in which `$( )`, backquotes, `!` and `\` are plain characters; a `'` is
/// written `'\''`. Control characters (a pasted newline would run the line)
/// are dropped.
pub fn shell_quote(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    format!("'{}'", clean.replace('\'', "'\\''"))
}

pub fn show(
    ui: &mut Ui,
    th: &Theme,
    snap: &Snapshot,
    now: u64,
    history: &Loader,
    range: &mut Range,
    rename: &mut Rename,
) {
    theme::scroll_body(ui, crate::shell::Tab::Data.label(), |ui| {
        let kb = snap.keyboard.clone().unwrap_or_default();
        th.panel(ui, tr("Battery History"), 0.0, |ui| {
            history_chart::show(ui, th, history, range);
        });
        ui.add_space(theme::GAP);
        if ui.available_width() >= theme::NARROW {
            theme::split(
                ui,
                0.5,
                |ui, _| batteries(ui, th, snap, &kb, now),
                |ui, _| {
                    device(ui, th, snap, &kb, rename);
                    ui.add_space(theme::GAP);
                    firmware(ui, th, &kb);
                },
            );
        } else {
            batteries(ui, th, snap, &kb, now);
            ui.add_space(theme::GAP);
            device(ui, th, snap, &kb, rename);
            ui.add_space(theme::GAP);
            firmware(ui, th, &kb);
        }
    });
}

fn batteries(ui: &mut Ui, th: &Theme, snap: &Snapshot, kb: &KbReport, now: u64) {
    th.panel(ui, tr("Batteries"), 0.0, |ui| {
        let b = &kb.battery;
        let dash = || view::DASH.to_string();
        // Measured: reports 0x46 / 0xFF, in mV (#139).
        let volts = b.voltage.filter(|v| v.is_finite() && *v > 0.0);
        th.kv(
            ui,
            tr("Voltage"),
            &volts.map_or_else(dash, view::volts_text),
            volts.map_or(Level::Unknown, view::voltage_level),
        );
        threshold_scale(ui, th, b);
        // Thresholds the keyboard reports (0x60, read once per connection).
        th.kv(
            ui,
            tr("Thresholds"),
            &view::thresholds_text(b).unwrap_or_else(dash),
            Level::Unknown,
        );
        // Charge estimated by the declared chemistry [hypothèse] (#178).
        th.kv(
            ui,
            tr("Estimate"),
            &view::estimate_text(b).unwrap_or_else(dash),
            Level::Unknown,
        );
        // What macOS would show for the same raw value (#213).
        th.kv(
            ui,
            tr("Apple display"),
            &view::apple_display_text(b).unwrap_or_else(dash),
            Level::Unknown,
        );
        th.kv(
            ui,
            tr("Chemistry"),
            &view::chemistry_text(b).unwrap_or_else(dash),
            Level::Unknown,
        );
        // The kernel % steps down only at reconnections (#179).
        th.kv(
            ui,
            tr("Updated"),
            &view::age_text(snap.update_age_s(now)),
            Level::Unknown,
        );
        th.kv(
            ui,
            tr("Remaining"),
            &view::remaining_text(snap, now).unwrap_or_else(dash),
            Level::Unknown,
        );
    });
}

/// The voltage on the scale of the thresholds the keyboard reports: Empty
/// and Full at the ends, Critical and Low as ticks, a needle on the value.
/// Nothing is drawn while the thresholds were not read.
fn threshold_scale(ui: &mut Ui, th: &Theme, b: &KbBattery) {
    let Some(t) = b.thresholds else { return };
    let mv = b.voltage_filtered_mv.or(b.voltage_mv).or_else(|| {
        b.voltage
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| (v * 1000.0) as u32)
    });
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
    let p = ui.painter();
    let (left, width) = (rect.left() + 8.0, (rect.width() - 16.0).max(1.0));
    let base = rect.top() + 20.0;
    p.hline(left..=left + width, base, Stroke::new(1.0, theme::PHOSPHOR));
    // Critical and Low are close to each other: Critical gets its own row.
    let marks = [
        (t.empty_mv, tr("Empty"), Level::Bad, Align2::LEFT_TOP, 0.0),
        (
            t.critical_mv,
            tr("Critical"),
            Level::Bad,
            Align2::CENTER_TOP,
            theme::SMALL,
        ),
        (t.low_mv, tr("Low"), Level::Warn, Align2::CENTER_TOP, 0.0),
        (t.full_mv, tr("Full"), Level::Good, Align2::RIGHT_TOP, 0.0),
    ];
    for (at, name, level, align, drop) in marks {
        let x = left + view::threshold_fraction(&t, u32::from(at)) * width;
        p.vline(
            x,
            base - 6.0..=base + 6.0 + drop,
            Stroke::new(1.5, theme::level_color(level)),
        );
        p.text(
            Pos2::new(x, base + 8.0 + drop),
            align,
            theme::caps(name),
            theme::font(theme::SMALL),
            theme::GREEN_MID,
        );
    }
    if let Some(mv) = mv {
        let x = left + view::threshold_fraction(&t, mv) * width;
        let color = theme::level_color(match t.level(mv) {
            akm_core::registry::ThresholdLevel::Ok => Level::Good,
            akm_core::registry::ThresholdLevel::Low => Level::Warn,
            _ => Level::Bad,
        });
        if th.crt {
            p.vline(
                x,
                base - 16.0..=base + 6.0,
                Stroke::new(7.0, theme::fade(color, 0.18)),
            );
        }
        p.vline(x, base - 16.0..=base + 6.0, Stroke::new(2.0, color));
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x - 5.0, base - 20.0),
                Pos2::new(x + 5.0, base - 20.0),
                Pos2::new(x, base - 12.0),
            ],
            color,
            Stroke::NONE,
        ));
    }
}

fn device(ui: &mut Ui, th: &Theme, snap: &Snapshot, kb: &KbReport, rename: &mut Rename) {
    th.panel(ui, tr("Device"), 0.0, |ui| {
        let d = &kb.device;
        for (key, value) in [
            (tr("Model"), &d.model),
            (tr("Own name"), &d.name),
            (tr("MAC"), &d.mac),
            (tr("Driver"), &d.driver),
            (tr("Paired host"), &kb.bluetooth.paired_host_addr),
            (tr("Chip"), &d.chip),
        ] {
            th.kv(
                ui,
                key,
                value.as_deref().unwrap_or(view::DASH),
                Level::Unknown,
            );
        }
        theme::rule(ui);
        theme::text(
            ui,
            &theme::caps(tr("Name on this computer")),
            theme::BODY,
            theme::GREEN_MID,
        );
        match d.mac.as_deref() {
            Some(mac) => rename_row(ui, th, mac, snap.display_name(), rename),
            None => {
                theme::text(ui, view::DASH, theme::BODY, theme::GREEN_MID);
            }
        }
        ui.add_space(4.0);
        theme::text(
            ui,
            tr("The name stored in the keyboard is changed from a terminal:"),
            theme::BODY,
            theme::GREEN_MID,
        );
        shell::command_line(ui, &device_name_command(&rename.buf));
    });
}

/// Editable name of the keyboard: text field + Rename / Reset (BlueZ alias,
/// through the daemon; nothing is written into the keyboard).
fn rename_row(ui: &mut Ui, th: &Theme, mac: &str, current: Option<&str>, st: &mut Rename) {
    let current = current.unwrap_or_default().to_string();
    let mut submit: Option<String> = None;
    let edit = ui.add(
        egui::TextEdit::singleline(&mut st.buf)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(6, 5))
            .char_limit(akm_core::alias::MAX_CHARS)
            .hint_text(tr("Keyboard name")),
    );
    if std::mem::take(&mut st.focus) {
        edit.request_focus();
        edit.scroll_to_me(Some(egui::Align::Center));
    }
    // Follow the daemon's name unless the user is typing.
    if st.loaded.as_deref() != Some(current.as_str()) && !edit.has_focus() {
        st.buf = current.clone();
        st.loaded = Some(current.clone());
    }
    let changed = st.buf.trim() != current;
    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.horizontal_wrapped(|ui| {
        if (theme::action(ui, tr("Rename"), changed).clicked() || enter) && changed {
            submit = Some(st.buf.clone());
        }
        if theme::action(ui, tr("Reset"), true)
            .on_hover_text(tr("Restore the keyboard's own name"))
            .clicked()
        {
            submit = Some(String::new());
        }
    });
    if let Some(text) = submit {
        let set = |v| *st.status.lock().unwrap_or_else(|e| e.into_inner()) = v;
        set(None);
        match rename::check(&text) {
            Ok(name) => rename::submit(mac.to_string(), name, st.status.clone(), ui.ctx().clone()),
            Err(e) => set(Some((false, e))),
        }
    }
    let status = st.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    shell::outcome(ui, th, &status);
}

fn firmware(ui: &mut Ui, th: &Theme, kb: &KbReport) {
    th.panel(ui, tr("Firmware"), 0.0, |ui| {
        // Firmware check against the embedded table (#227); never a flash offer.
        match view::firmware_line(&kb.firmware, crate::i18n::is_french()) {
            Some((line, level)) => {
                theme::text(ui, &line, theme::BODY, theme::value_color(level));
                if let Some(src) = kb.firmware.source.as_deref() {
                    theme::text(
                        ui,
                        &trf(
                            "Source: {} \u{b7} table of {}",
                            &[&src, &kb.firmware.table_date.as_deref().unwrap_or("?")],
                        ),
                        theme::SMALL,
                        theme::GREEN_MID,
                    );
                }
            }
            None => {
                theme::text(
                    ui,
                    tr("Firmware: not read yet (read once per connection)"),
                    theme::BODY,
                    theme::GREEN_MID,
                );
            }
        }
        // Uninterpreted vendor reports (meaning not proven, #131/#132).
        for (id, hex) in &kb.raw {
            th.kv(ui, &trf("{} (raw)", &[id]), hex, Level::Unknown);
        }
        if kb.incomplete {
            th.kv(ui, tr("Read"), tr("incomplete (timeout)"), Level::Warn);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_command_is_shown_never_run() {
        assert_eq!(
            device_name_command("Bureau"),
            "akmctl rename --device-name 'Bureau'"
        );
        assert_eq!(
            device_name_command("  "),
            "akmctl rename --device-name 'NAME'"
        );
        // Nothing that would end the quoted argument.
        assert_eq!(
            device_name_command("a\"; rm -rf \\"),
            "akmctl rename --device-name 'a\"; rm -rf \\'"
        );
    }

    /// #276: the name comes from the keyboard itself (its announced name);
    /// pasted in a terminal, the command must run nothing else.
    #[test]
    fn device_name_command_never_expands_in_a_shell() {
        for evil in [
            "$(touch PWNED)",
            "`touch PWNED`",
            "x'; touch PWNED; echo '",
            "!! ${HOME} \\$(id)",
            "a\ntouch PWNED",
        ] {
            let cmd = device_name_command(evil);
            assert!(!cmd.contains('\n'), "{cmd}");
            // What a real shell sees: exactly 4 words, the name intact.
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("set -- {cmd}; printf '%s\\n' \"$#\" \"$4\""))
                .output()
                .unwrap();
            let out = String::from_utf8_lossy(&out.stdout);
            let want: String = evil.trim().chars().filter(|c| !c.is_control()).collect();
            assert_eq!(out, format!("4\n{want}\n"), "{cmd}");
        }
    }
}
