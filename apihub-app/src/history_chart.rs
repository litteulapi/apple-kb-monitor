//! Drawing of the battery history (extracted from `main.rs`, #62), in the
//! DATA tab: period selector, then the chart as a CRT trace.

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::history_view::Loader;
use crate::i18n::tr;
use crate::theme::{self, Theme};
use crate::view::Range;

/// Height of the chart, and its margins around the plot: the percentage
/// labels on the left, the legend on top, the time ticks at the bottom.
const HEIGHT: f32 = 190.0;
const HEIGHT_NARROW: f32 = 170.0;
const MARGIN_LEFT: f32 = 44.0;
const MARGIN_TOP: f32 = 26.0;
const MARGIN_BOTTOM: f32 = 24.0;
const MARGIN_RIGHT: f32 = 10.0;

/// Reload the history (Refresh button, F5).
pub fn reload(ctx: &egui::Context, history: &Loader) {
    let ctx = ctx.clone();
    history.request(move || ctx.request_repaint());
}

/// Battery + voltage history chart over the chosen period: 24 h, 7, 30 or
/// 90 days (#96). Pure drawing: the data is loaded by a worker thread and
/// laid out by `crate::view::chart_model`, which bounds the number of
/// points, so a frame never waits on D-Bus or the disk and never lays out an
/// unbounded string (#230).
pub fn show(ui: &mut Ui, th: &Theme, history: &Loader, range: &mut Range) {
    let data = history.data();
    ui.horizontal_wrapped(|ui| {
        for r in Range::ALL {
            if theme::choice(ui, *range == r, r.button())
                .on_hover_text(theme::glyphs(r.text()))
                .clicked()
            {
                *range = r;
            }
        }
        if theme::action(ui, tr("Refresh"), !data.loading).clicked() {
            reload(ui.ctx(), history);
        }
        if data.loading {
            theme::text(ui, tr("Loading…"), theme::BODY, theme::AMBER);
        }
    });
    if let Some(note) = &data.note {
        theme::text(ui, note, theme::SMALL, theme::AMBER);
    }

    let model = match crate::view::chart_model(
        &data.battery,
        &data.voltage,
        crate::unix_now() as f64,
        *range,
    ) {
        Ok(m) => m,
        Err(msg) => {
            let msg = if data.loading && data.battery.is_empty() {
                tr("Loading history...")
            } else {
                msg.as_str()
            };
            theme::text(ui, &format!("> {msg} _"), theme::BODY, theme::GREEN_MID);
            return;
        }
    };
    theme::text(ui, &model.summary, theme::SMALL, theme::GREEN_MID);

    let narrow = ui.available_width() < theme::NARROW;
    let (response, painter) = ui.allocate_painter(
        Vec2::new(
            ui.available_width(),
            if narrow { HEIGHT_NARROW } else { HEIGHT },
        ),
        Sense::hover(),
    );
    let rect = response.rect;
    painter.rect(
        rect.shrink(0.5),
        0.0,
        theme::BG,
        Stroke::new(1.0, theme::GREEN_FRAME),
        egui::StrokeKind::Inside,
    );
    let plot = Rect::from_min_max(
        Pos2::new(rect.min.x + MARGIN_LEFT, rect.min.y + MARGIN_TOP),
        Pos2::new(rect.max.x - MARGIN_RIGHT, rect.max.y - MARGIN_BOTTOM),
    );
    if plot.width() < 10.0 || plot.height() < 10.0 {
        return;
    }
    let painter = painter.with_clip_rect(rect);
    let (t_min, t_range) = (model.t_min, (model.t_max - model.t_min).max(1.0));
    let x_of =
        |ts: f64| plot.min.x + ((ts - t_min) / t_range).clamp(0.0, 1.0) as f32 * plot.width();
    let y_of = |frac: f64| plot.max.y - (frac.clamp(0.0, 1.0) as f32) * plot.height();
    let grid = Stroke::new(1.0, theme::fade(theme::GREEN_FRAME, 0.55));
    let small = theme::font(theme::SMALL);

    for level in [0.0, 25.0, 50.0, 75.0, 100.0] {
        let y = y_of(level / 100.0);
        painter.hline(plot.x_range(), y, grid);
        painter.text(
            Pos2::new(plot.min.x - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{level:.0}%"),
            small.clone(),
            theme::GREEN_MID,
        );
    }
    // Tick labels never overlap: one is skipped when it would run into the
    // previous one.
    let mut label_end = f32::NEG_INFINITY;
    for (t, label) in &model.x_ticks {
        let x = x_of(*t);
        painter.vline(x, plot.y_range(), grid);
        let w = label.chars().count() as f32 * theme::ADVANCE * theme::SMALL;
        let (align, left) = if x + w / 2.0 > plot.max.x {
            (Align2::RIGHT_TOP, x - w)
        } else if x - w / 2.0 < plot.min.x {
            (Align2::LEFT_TOP, x)
        } else {
            (Align2::CENTER_TOP, x - w / 2.0)
        };
        if left < label_end + theme::GAP {
            continue;
        }
        label_end = left + w;
        painter.text(
            Pos2::new(x, plot.max.y + 3.0),
            align,
            theme::glyphs(label),
            small.clone(),
            theme::GREEN_MID,
        );
    }

    // Voltage first (thin, secondary green), the battery trace over it.
    if let Some((v_lo, v_hi)) = model.volt_axis {
        let line: Vec<Pos2> = model
            .volt
            .iter()
            .map(|&(t, v)| Pos2::new(x_of(t), y_of((v - v_lo) / (v_hi - v_lo))))
            .collect();
        painter.extend(egui::Shape::dashed_line(
            &line,
            Stroke::new(1.5, theme::GREEN_MID),
            6.0,
            4.0,
        ));
    }
    let line: Vec<Pos2> = model
        .batt
        .iter()
        .map(|&(t, p)| Pos2::new(x_of(t), y_of(p / 100.0)))
        .collect();
    if th.crt {
        painter.add(egui::Shape::line(
            line.clone(),
            Stroke::new(6.0, theme::fade(theme::PHOSPHOR, 0.16)),
        ));
    }
    painter.add(egui::Shape::line(line, Stroke::new(2.0, theme::PHOSPHOR)));

    // Legend: swatch then its (bounded) text, laid out right to left.
    let inks = [theme::PHOSPHOR, theme::GREEN_MID];
    let mut x = rect.max.x - MARGIN_RIGHT;
    for (i, (text, ink)) in model.legend.iter().zip(inks).enumerate().rev() {
        let galley = painter.layout_no_wrap(theme::glyphs(text).into_owned(), small.clone(), ink);
        x -= galley.size().x;
        if x - 30.0 < rect.min.x {
            break;
        }
        let y = rect.min.y + 4.0;
        let mid = y + galley.size().y / 2.0;
        painter.galley(Pos2::new(x, y), galley, ink);
        let swatch = [Pos2::new(x - 22.0, mid), Pos2::new(x - 6.0, mid)];
        if i == 0 {
            painter.line_segment(swatch, Stroke::new(2.0, ink));
        } else {
            painter.extend(egui::Shape::dashed_line(
                &swatch,
                Stroke::new(1.5, ink),
                6.0,
                4.0,
            ));
        }
        x -= 22.0 + 2.0 * theme::GAP;
    }
}
