//! Drawing of the battery history tile (extracted from `main.rs`, #62).

use eframe::egui;

use crate::history_view::Loader;
use crate::i18n::tr;
use crate::view::Palette;
use crate::widgets::tile;

/// Battery + voltage history chart (last 24 h). Pure drawing: the data
/// is loaded by a worker thread and laid out by `crate::view::chart_model`, so a
/// frame never waits on D-Bus or the disk and never lays out an
/// unbounded string (#230).
pub fn show(ui: &mut egui::Ui, history: &Loader, palette: &Palette) {
    let data = history.data();
    tile(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr("Battery History (24 h)"))
                    .strong()
                    .size(18.0),
            );
            let refresh = ui.add_enabled(
                !data.loading,
                egui::Button::new(egui::RichText::new(tr("Refresh")).size(14.0)),
            );
            if refresh.clicked() {
                let ctx = ui.ctx().clone();
                history.request(move || ctx.request_repaint());
            }
            if data.loading {
                ui.spinner();
            }
        });
        if let Some(note) = &data.note {
            ui.add(egui::Label::new(egui::RichText::new(note.as_str()).weak().size(14.0)).wrap());
        }

        let model = match crate::view::chart_model(
            &data.battery,
            &data.voltage,
            crate::unix_now() as f64,
        ) {
            Ok(m) => m,
            Err(msg) => {
                let msg = if data.loading && data.battery.is_empty() {
                    tr("Loading history...")
                } else {
                    msg.as_str()
                };
                ui.label(egui::RichText::new(msg).weak().size(16.0));
                return;
            }
        };
        ui.label(
            egui::RichText::new(model.summary.as_str())
                .weak()
                .size(14.0),
        );
        ui.add_space(4.0);

        let (response, painter) = ui.allocate_painter(
            egui::Vec2::new(ui.available_width(), 170.0),
            egui::Sense::hover(),
        );
        let rect = response.rect;
        let vis = ui.visuals().clone();
        painter.rect_filled(rect, 4.0, vis.extreme_bg_color);
        // Room on the left for the % labels, on top for the legend,
        // at the bottom for the time ticks.
        let plot_rect = egui::Rect::from_min_max(
            egui::Pos2::new(rect.min.x + 36.0, rect.min.y + 22.0),
            egui::Pos2::new(rect.max.x - 8.0, rect.max.y - 20.0),
        );
        if plot_rect.width() < 10.0 || plot_rect.height() < 10.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect);
        let (t_min, t_range) = (model.t_min, (model.t_max - model.t_min).max(1.0));
        let x_of = |ts: f64| {
            plot_rect.min.x + ((ts - t_min) / t_range).clamp(0.0, 1.0) as f32 * plot_rect.width()
        };
        let y_of = |frac: f64| plot_rect.max.y - (frac.clamp(0.0, 1.0) as f32) * plot_rect.height();
        let grid = egui::Stroke::new(0.5, vis.widgets.noninteractive.bg_stroke.color);
        let small = egui::FontId::proportional(10.0);

        for level in [0.0, 25.0, 50.0, 75.0, 100.0] {
            let y = y_of(level / 100.0);
            painter.line_segment(
                [
                    egui::Pos2::new(plot_rect.min.x, y),
                    egui::Pos2::new(plot_rect.max.x, y),
                ],
                grid,
            );
            painter.text(
                egui::Pos2::new(plot_rect.min.x - 4.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{level:.0}%"),
                small.clone(),
                vis.weak_text_color(),
            );
        }
        for (t, label) in &model.x_ticks {
            let x = x_of(*t);
            painter.line_segment(
                [
                    egui::Pos2::new(x, plot_rect.min.y),
                    egui::Pos2::new(x, plot_rect.max.y),
                ],
                grid,
            );
            let align = if x > plot_rect.max.x - 30.0 {
                egui::Align2::RIGHT_TOP
            } else if x < plot_rect.min.x + 30.0 {
                egui::Align2::LEFT_TOP
            } else {
                egui::Align2::CENTER_TOP
            };
            painter.text(
                egui::Pos2::new(x, plot_rect.max.y + 4.0),
                align,
                label,
                small.clone(),
                vis.weak_text_color(),
            );
        }

        let batt_color = palette.good;
        let line: Vec<egui::Pos2> = model
            .batt
            .iter()
            .map(|&(t, p)| egui::Pos2::new(x_of(t), y_of(p / 100.0)))
            .collect();
        painter.add(egui::Shape::line(line, egui::Stroke::new(2.0, batt_color)));

        let volt_color = palette.info;
        if let Some((v_lo, v_hi)) = model.volt_axis {
            let line: Vec<egui::Pos2> = model
                .volt
                .iter()
                .map(|&(t, v)| egui::Pos2::new(x_of(t), y_of((v - v_lo) / (v_hi - v_lo))))
                .collect();
            painter.add(egui::Shape::line(line, egui::Stroke::new(1.5, volt_color)));
        }

        // Legend: swatch then its (bounded) text, laid out right to left.
        let colors = [batt_color, volt_color];
        let mut x = rect.max.x - 8.0;
        for (text, color) in model.legend.iter().zip(colors).rev() {
            let galley = painter.layout_no_wrap(
                text.clone(),
                egui::FontId::proportional(11.0),
                vis.weak_text_color(),
            );
            x -= galley.size().x;
            let y = rect.min.y + 5.0;
            painter.galley(egui::Pos2::new(x, y), galley, vis.weak_text_color());
            x -= 12.0;
            painter.rect_filled(
                egui::Rect::from_min_size(egui::Pos2::new(x, y + 2.0), egui::Vec2::splat(8.0)),
                1.0,
                color,
            );
            x -= 14.0;
        }
    });
}
