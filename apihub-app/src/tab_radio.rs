//! RADIO tab: the Bluetooth link. Composition: a tuner scale carrying the
//! signal reading (relative dB, weak / good / excellent zones), the link
//! facts, and the two ways to bring the link back (reconnect through the
//! daemon; `akmctl repair` in a terminal, never run by the window).

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use akm_core::report::KbReport;
use akm_core::Snapshot;

use crate::actions::Job;
use crate::i18n::tr;
use crate::shell;
use crate::theme::{self, Theme};
use crate::view::{self, Level};

/// The command that removes and redoes the pairing (typed confirmation).
pub const REPAIR_COMMAND: &str = "akmctl repair";

pub fn show(ui: &mut Ui, th: &Theme, snap: &Snapshot, now: u64, link: &Job) {
    theme::scroll_body(ui, crate::shell::Tab::Radio.label(), |ui| {
        let kb = snap.keyboard.clone().unwrap_or_default();
        th.panel(ui, tr("Signal"), 0.0, |ui| tuner(ui, th, snap, &kb, now));
        ui.add_space(theme::GAP);
        if ui.available_width() >= theme::NARROW {
            theme::split(
                ui,
                0.5,
                |ui, _| link_panel(ui, th, snap, &kb),
                |ui, _| repair_panel(ui, th, link),
            );
        } else {
            link_panel(ui, th, snap, &kb);
            ui.add_space(theme::GAP);
            repair_panel(ui, th, link);
        }
    });
}

/// Zones of the tuner: first relative dB of the zone and its name.
fn zones() -> [(i32, &'static str); 3] {
    [
        (view::TUNER_MIN_DB, tr("weak")),
        (-5, tr("good")),
        (0, tr("excellent")),
    ]
}

/// X of a relative-dB value on a scale drawn from `left` over `width`.
pub fn scale_x(db: i32, left: f32, width: f32) -> f32 {
    let span = (view::TUNER_MAX_DB - view::TUNER_MIN_DB) as f32;
    let f = (db.clamp(view::TUNER_MIN_DB, view::TUNER_MAX_DB) - view::TUNER_MIN_DB) as f32 / span;
    left + f * width
}

fn tuner(ui: &mut Ui, th: &Theme, snap: &Snapshot, kb: &KbReport, now: u64) {
    // Relative BR/EDR value (dB to the ideal range), not dBm (#174).
    let rssi = kb.radio.rel_db();
    let level = view::rssi_level(rssi);
    let color = theme::level_color(level);
    let avail = ui.available_width();
    // Reading: the quality in words, and the four bars.
    let (head, resp) = ui.allocate_exact_size(Vec2::new(avail, 44.0), Sense::hover());
    let said = view::rssi_text(rssi);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
    let p = ui.painter();
    let size = theme::fit_size(
        said.chars().count(),
        avail - 70.0,
        &[44.0, 36.0, theme::VALUE],
    );
    th.glow(
        p,
        head.left_center(),
        Align2::LEFT_CENTER,
        &theme::caps(&said),
        size,
        color,
    );
    theme::signal_bars(
        p,
        Rect::from_min_size(
            Pos2::new(head.right() - 52.0, head.top() + 6.0),
            Vec2::new(52.0, 30.0),
        ),
        view::rssi_bar_count(rssi),
        color,
    );
    // Scale: zones above the line, ticks under it, the needle on the value.
    let (r, _) = ui.allocate_exact_size(Vec2::new(avail, 88.0), Sense::hover());
    let p = ui.painter();
    let (left, width) = (r.left() + 14.0, (r.width() - 28.0).max(1.0));
    let base = r.top() + 54.0;
    p.hline(left..=left + width, base, Stroke::new(1.0, theme::PHOSPHOR));
    let z = zones();
    for (i, (from, name)) in z.iter().enumerate() {
        let to = z.get(i + 1).map_or(view::TUNER_MAX_DB, |n| n.0);
        let (x0, x1) = (scale_x(*from, left, width), scale_x(to, left, width));
        p.vline(x0, base - 34.0..=base, Stroke::new(1.0, theme::GREEN_FRAME));
        let name = theme::caps(name);
        if name.chars().count() as f32 * theme::ADVANCE * theme::SMALL <= x1 - x0 {
            p.text(
                Pos2::new((x0 + x1) / 2.0, base - 34.0),
                Align2::CENTER_BOTTOM,
                name,
                theme::font(theme::SMALL),
                theme::GREEN_MID,
            );
        }
    }
    for db in view::TUNER_MIN_DB..=view::TUNER_MAX_DB {
        let x = scale_x(db, left, width);
        let major = db % 5 == 0;
        p.vline(
            x,
            base..=base + if major { 9.0 } else { 4.0 },
            Stroke::new(1.0, theme::GREEN_MID),
        );
        if major && (width >= 300.0 || db % 10 == 0) {
            p.text(
                Pos2::new(x, base + 11.0),
                Align2::CENTER_TOP,
                akm_core::signal::raw_text(db),
                theme::font(theme::SMALL),
                theme::GREEN_MID,
            );
        }
    }
    if let Some(f) = view::tuner_fraction(rssi) {
        let x = left + f * width;
        if th.crt {
            p.vline(
                x,
                base - 22.0..=base + 8.0,
                Stroke::new(7.0, theme::fade(color, 0.18)),
            );
        }
        p.vline(x, base - 22.0..=base + 8.0, Stroke::new(2.0, color));
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x - 6.0, base - 30.0),
                Pos2::new(x + 6.0, base - 30.0),
                Pos2::new(x, base - 20.0),
            ],
            color,
            Stroke::NONE,
        ));
    }
    let measured = match (view::rssi_valid(rssi), snap.rssi_age_s(now)) {
        (Some(_), Some(age)) => view::age_text(Some(age)),
        _ => view::DASH.into(),
    };
    th.kv(ui, tr("Measured"), &measured, Level::Unknown);
}

fn link_panel(ui: &mut Ui, th: &Theme, snap: &Snapshot, kb: &KbReport) {
    th.panel(ui, tr("Link"), 0.0, |ui| {
        let (text, level) = match (&snap.keyboard, kb.bluetooth.connected) {
            (None, _) => (view::DASH, Level::Unknown),
            (Some(_), true) => (tr("Yes"), Level::Good),
            (Some(_), false) => (tr("No"), Level::Bad),
        };
        th.kv(ui, tr("Connected"), text, level);
        th.kv(
            ui,
            tr("Paired"),
            view::paired_text(&kb.bluetooth).unwrap_or(view::DASH),
            Level::Unknown,
        );
        th.kv(
            ui,
            tr("TX Power"),
            &view::tx_power_text(kb.radio.tx_power_dbm),
            Level::Unknown,
        );
        // Passive listening of input report 0x13.
        th.kv(
            ui,
            tr("Last wake"),
            &view::wake_text(&kb.wake).unwrap_or_else(|| view::DASH.into()),
            Level::Unknown,
        );
        th.kv(
            ui,
            tr("Paired host"),
            kb.bluetooth
                .paired_host_addr
                .as_deref()
                .unwrap_or(view::DASH),
            Level::Unknown,
        );
    });
}

fn repair_panel(ui: &mut Ui, th: &Theme, link: &Job) {
    th.panel(ui, tr("Restore the link"), 0.0, |ui| {
        ui.horizontal_wrapped(|ui| shell::reconnect_button(ui, link));
        shell::outcome(ui, th, &link.outcome());
        ui.add_space(4.0);
        theme::text(
            ui,
            tr("Pairing lost or refused? Redo it from a terminal (asks for a confirmation):"),
            theme::BODY,
            theme::GREEN_MID,
        );
        shell::command_line(ui, REPAIR_COMMAND);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_maps_the_shown_range_and_clamps() {
        assert_eq!(scale_x(view::TUNER_MIN_DB, 10.0, 300.0), 10.0);
        assert_eq!(scale_x(view::TUNER_MAX_DB, 10.0, 300.0), 310.0);
        assert_eq!(scale_x(-90, 10.0, 300.0), 10.0);
        assert_eq!(scale_x(40, 10.0, 300.0), 310.0);
        assert_eq!(scale_x(-10, 0.0, 250.0), 100.0);
        // The needle and the scale agree.
        let f = view::tuner_fraction(Some(-3)).unwrap();
        assert!((scale_x(-3, 0.0, 300.0) - f * 300.0).abs() < 1e-3);
    }

    /// The zones are the ones of `akm_core::signal::quality`.
    #[test]
    fn zones_follow_the_quality_thresholds() {
        use akm_core::signal::{quality, SignalQuality as Q};
        let z = zones();
        assert_eq!(z[0].0, view::TUNER_MIN_DB);
        assert_eq!((quality(z[1].0 - 1), quality(z[1].0)), (Q::Weak, Q::Good));
        assert_eq!(
            (quality(z[2].0 - 1), quality(z[2].0)),
            (Q::Good, Q::Excellent)
        );
        assert_eq!(z.map(|z| z.1), ["weak", "good", "excellent"]);
    }
}
