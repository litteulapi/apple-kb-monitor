//! STAT tab: everything at a glance (batteries, link, alert) and the three
//! everyday actions. Composition: the huge battery percentage and its
//! block gauge, the keyboard drawn as the "condition figure" of the device,
//! a strip of four stat cells, then the actions.

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use akm_core::report::KbReport;
use akm_core::Snapshot;

use crate::actions::{FnMode, Job};
use crate::i18n::{tr, trf};
use crate::shell;
use crate::theme::{self, Theme};
use crate::view::{self, Level};

/// Width / height of the keyboard figure (an A1314 is 281 x 115 mm).
const FIGURE_ASPECT: f32 = 2.45;
const FIGURE_MAX_HEIGHT: f32 = 240.0;
/// Share of the width given to the batteries when the two top panels sit
/// side by side.
const BATTERY_SHARE: f32 = 0.42;
/// Largest battery percentage (very large windows).
const HERO_MAX: f32 = 260.0;
/// Heights the layout reserves: what a panel adds around its content, the
/// lines of the battery panel under the number (gauge, caption), the lines
/// of the keyboard panel around the figure and its name (spacing,
/// indicators), the strip of cells and the action panel with one line for
/// the outcome of an action.
const PANEL_CHROME: f32 = 34.0;
const BATTERY_LINES: f32 = 60.0;
const KEYBOARD_LINES: f32 = 50.0;
const CELLS_HEIGHT: f32 = 66.0;
const ACTIONS_HEIGHT: f32 = 96.0;
/// Largest empty band between the cells and the actions.
const ACTIONS_DROP_MAX: f32 = 120.0;
/// Share of its size a line of the battery number really occupies.
const HERO_LINE: f32 = 0.74;

/// What the tab asks of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Open DATA with the name field focused.
    Rename,
}

/// Sizes of the wide layout for a tab body `width` x `height` and a keyboard
/// name `name` tall (it wraps when long): the content
/// height of the two top panels, the size of the battery number and the
/// height of the keyboard figure. The top row takes what the figure needs,
/// never more than the body can show without scrolling, never less than
/// the smallest number needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WideLayout {
    pub top: f32,
    pub hero: f32,
    pub figure: f32,
}

pub fn wide_layout(width: f32, height: f32, name: f32) -> WideLayout {
    let left = ((width - theme::GAP) * BATTERY_SHARE).floor() - 24.0;
    let right = keyboard_width(width);
    let floor = theme::HERO_NARROW * HERO_LINE + BATTERY_LINES;
    let room = height - CELLS_HEIGHT - ACTIONS_HEIGHT - 2.0 * theme::GAP - PANEL_CHROME;
    let around = KEYBOARD_LINES + name;
    let natural = (right / FIGURE_ASPECT).min(FIGURE_MAX_HEIGHT) + around;
    let top = natural.min(room).max(floor);
    // "100 %" must fit in the panel: three digits and the unit.
    let by_width = left / (3.0 * theme::ADVANCE + 0.42 * theme::ADVANCE + 0.08);
    let by_height = (top - BATTERY_LINES) / HERO_LINE;
    let hero = by_height.min(by_width).clamp(theme::HERO_NARROW, HERO_MAX);
    let figure = (top - around).clamp(40.0, FIGURE_MAX_HEIGHT);
    WideLayout { top, hero, figure }
}

pub fn show(
    ui: &mut Ui,
    th: &Theme,
    snap: &Snapshot,
    now: u64,
    link: &Job,
    fnmode: &FnMode,
) -> Option<Request> {
    let mut request = None;
    let body = ui.available_size();
    theme::scroll_body(ui, crate::shell::Tab::Stat.label(), |ui| {
        let kb = snap.keyboard.clone().unwrap_or_default();
        let top = ui.cursor().top();
        if ui.available_width() >= theme::NARROW {
            let width = ui.available_width();
            let l = wide_layout(width, body.y, name_height(ui, snap, width));
            theme::split(
                ui,
                BATTERY_SHARE,
                |ui, _| battery_panel(ui, th, &kb, l.hero, l.top),
                |ui, _| keyboard_panel(ui, th, snap, &kb, l.top, l.figure, true),
            );
            ui.add_space(theme::GAP);
            cells(ui, th, snap, &kb, now, 4);
            // The actions sit at the bottom of the screen when there is room
            // (on a very tall window they stay near the figures).
            let used = ui.cursor().top() - top;
            ui.add_space((body.y - used - ACTIONS_HEIGHT).clamp(theme::GAP, ACTIONS_DROP_MAX));
        } else {
            // Narrow: the figures, then the actions, all on the first
            // screen at 420 x 700; the drawing comes last, reached by
            // scrolling (arrows, page keys, wheel). The name of the keyboard
            // stays in the status bar.
            battery_panel(ui, th, &kb, theme::HERO_NARROW, 0.0);
            ui.add_space(theme::GAP);
            cells(ui, th, snap, &kb, now, 2);
            ui.add_space(theme::GAP);
            request = actions(ui, th, snap, link, fnmode);
            ui.add_space(theme::GAP);
            keyboard_panel(ui, th, snap, &kb, 0.0, FIGURE_MAX_HEIGHT, false);
            return;
        }
        request = actions(ui, th, snap, link, fnmode);
    });
    request
}

/// Width of the content of the keyboard panel in a body `width` wide.
fn keyboard_width(width: f32) -> f32 {
    width - theme::GAP - ((width - theme::GAP) * BATTERY_SHARE).floor() - 24.0
}

/// Height of the keyboard name once wrapped in its panel.
fn name_height(ui: &Ui, snap: &Snapshot, width: f32) -> f32 {
    let name = snap.display_name().unwrap_or(view::DASH);
    ui.painter()
        .layout(
            theme::glyphs(name).into_owned(),
            theme::font(theme::BODY),
            theme::PHOSPHOR,
            keyboard_width(width),
        )
        .size()
        .y
}

/// The percentage as a number and its unit; unknown is `---` without unit.
pub fn hero_text(pct: Option<f64>) -> (String, &'static str) {
    match pct.filter(|v| v.is_finite()) {
        Some(v) => (format!("{v:.0}"), "%"),
        None => (view::DASH.to_string(), ""),
    }
}

fn battery_panel(ui: &mut Ui, th: &Theme, kb: &KbReport, size: f32, min_height: f32) {
    th.panel(ui, tr("Batteries"), min_height, |ui| {
        // Kernel / 0x47 first; 0xEA is not a percentage (#136). An estimate
        // is said to be one (#198).
        let src = view::pct_source(&kb.battery);
        let pct = src.value();
        let color = theme::level_color(view::battery_level(pct));
        let (number, unit) = hero_text(pct);
        // The glyphs of VT323 leave the top 15 % of the line empty.
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), size * HERO_LINE),
            Sense::hover(),
        );
        let p = ui.painter();
        let top = rect.top() - size * 0.17;
        // ... and their left bearing would shift the number off the gauge.
        let n = th.glow(
            p,
            Pos2::new(rect.left() - size * 0.05, top),
            Align2::LEFT_TOP,
            &number,
            size,
            color,
        );
        if !unit.is_empty() {
            th.glow(
                p,
                Pos2::new(n.right() + 6.0, n.bottom() - size * 0.2),
                Align2::LEFT_BOTTOM,
                unit,
                size * 0.42,
                color,
            );
        }
        let said = view::pct_text(pct, 0);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
        ui.add_space(4.0);
        let count = theme::segment_count(ui.available_width());
        let lit = theme::lit_segments(view::pct_fraction(pct), count);
        theme::segments(ui, 20.0, count, |i| (i < lit).then_some(color));
        ui.add_space(2.0);
        th.kv(
            ui,
            src.caption(),
            &view::pct_text(pct, 1),
            view::battery_level(pct),
        );
    });
}

fn keyboard_panel(
    ui: &mut Ui,
    th: &Theme,
    snap: &Snapshot,
    kb: &KbReport,
    min_height: f32,
    figure_height: f32,
    named: bool,
) {
    th.panel(ui, tr("Keyboard"), min_height, |ui| {
        let online = view::link_up(snap);
        let name = snap.display_name().unwrap_or(view::DASH);
        if named {
            let ink = if snap.display_name().is_some() {
                theme::PHOSPHOR
            } else {
                theme::GREEN_MID
            };
            theme::text(ui, name, theme::BODY, ink);
            ui.add_space(2.0);
        }
        let w = ui.available_width();
        let h = (w / FIGURE_ASPECT).min(figure_height);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        let fig = Rect::from_center_size(rect.center(), Vec2::new(h * FIGURE_ASPECT, h));
        let pct = view::pct_source(&kb.battery).value();
        figure(ui.painter(), th, fig, pct, snap.caps_lock, online);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, name));
        ui.add_space(4.0);
        leds(ui, snap);
    });
}

/// Unit widths of the keys, row by row (function row first).
const KEY_ROWS: [&[f32]; 6] = [
    &[1.0; 14],
    &[
        1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.5,
    ],
    &[
        1.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    ],
    &[
        1.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.75,
    ],
    &[
        1.25, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.25,
    ],
    &[1.0, 1.0, 1.0, 1.25, 5.25, 1.25, 1.0, 1.0, 1.0, 1.0],
];
/// The caps-lock key in [`KEY_ROWS`].
const CAPS_KEY: (usize, usize) = (3, 0);

/// Rectangles of the keys of the figure inside `area`: `(row, column, rect)`.
pub fn key_rects(area: Rect) -> Vec<(usize, usize, Rect)> {
    let gap = 2.0;
    // The function row is half as tall as the five others.
    let unit_h = (area.height() - 5.0 * gap) / 5.5;
    let mut out = Vec::new();
    let mut y = area.top();
    for (r, row) in KEY_ROWS.iter().enumerate() {
        let h = if r == 0 { unit_h * 0.5 } else { unit_h };
        let units: f32 = row.iter().sum();
        let unit_w = (area.width() - (row.len() - 1) as f32 * gap) / units;
        let mut x = area.left();
        for (c, u) in row.iter().enumerate() {
            let w = u * unit_w;
            out.push((r, c, Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))));
            x += w + gap;
        }
        y += h + gap;
    }
    out
}

/// The keyboard as a line drawing: battery tube with its two cells filled
/// to the charge, the keys (caps lock lit when on), dimmed when offline.
fn figure(p: &egui::Painter, th: &Theme, r: Rect, pct: Option<f64>, caps: bool, online: bool) {
    if r.width() < 40.0 || r.height() < 20.0 {
        return;
    }
    let ink = if online {
        theme::PHOSPHOR
    } else {
        theme::GREEN_FRAME
    };
    let line = Stroke::new(1.5, ink);
    // Battery tube along the back edge.
    let tube_h = r.height() * 0.2;
    let tube = Rect::from_min_size(r.min, Vec2::new(r.width(), tube_h));
    p.rect_stroke(tube, tube_h / 2.0, line, StrokeKind::Inside);
    let level = view::battery_level(pct);
    let frac = view::pct_fraction(pct);
    let cell_w = r.width() * 0.17;
    let cell_h = tube_h - 8.0;
    for i in 0..2 {
        let cell = Rect::from_min_size(
            Pos2::new(
                tube.left() + tube_h * 0.6 + i as f32 * (cell_w + 6.0),
                tube.top() + 4.0,
            ),
            Vec2::new(cell_w, cell_h),
        );
        p.rect_stroke(
            cell,
            0.0,
            Stroke::new(1.0, theme::GREEN_MID),
            StrokeKind::Inside,
        );
        if frac > 0.0 {
            let fill = Rect::from_min_size(
                cell.min + Vec2::splat(2.0),
                Vec2::new((cell.width() - 4.0) * frac, cell.height() - 4.0),
            );
            p.rect_filled(fill, 0.0, theme::level_color(level));
        }
        // Positive terminal.
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(cell.right(), cell.center().y - 2.0),
                Vec2::new(2.0, 4.0),
            ),
            0.0,
            theme::GREEN_MID,
        );
    }
    // Power button at the right end of the tube.
    p.circle_stroke(
        Pos2::new(tube.right() - tube_h * 0.5, tube.center().y),
        tube_h * 0.22,
        Stroke::new(1.0, ink),
    );
    // Body and keys.
    let body = Rect::from_min_max(Pos2::new(r.left(), tube.bottom() + 3.0), r.max);
    p.rect_stroke(body, 5.0, line, StrokeKind::Inside);
    let key_ink = if online {
        theme::GREEN_DIM
    } else {
        theme::GREEN_FRAME
    };
    for (row, col, k) in key_rects(body.shrink(6.0)) {
        if online && caps && (row, col) == CAPS_KEY {
            p.rect_filled(k, 1.0, theme::PHOSPHOR);
        } else {
            p.rect_stroke(k, 1.0, Stroke::new(1.0, key_ink), StrokeKind::Inside);
        }
    }
    if !online {
        let word = tr("OFFLINE");
        let w = word.chars().count() as f32 * theme::ADVANCE * theme::VALUE + 24.0;
        let plate = Rect::from_center_size(body.center(), Vec2::new(w, theme::VALUE + 8.0));
        p.rect(
            plate,
            0.0,
            theme::BG,
            Stroke::new(1.5, theme::RED),
            StrokeKind::Inside,
        );
        th.glow(
            p,
            plate.center(),
            Align2::CENTER_CENTER,
            word,
            theme::VALUE,
            theme::RED,
        );
    }
}

/// Lock indicators of the keyboard: a lit or empty square and its name.
fn leds(ui: &mut Ui, snap: &Snapshot) {
    ui.horizontal(|ui| {
        theme::text(ui, &theme::caps(tr("LEDs")), theme::BODY, theme::GREEN_MID);
        for (name, on) in [(tr("CAPS"), snap.caps_lock), (tr("NUM"), snap.num_lock)] {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            let led = rect.translate(Vec2::new(0.0, 1.0));
            if on {
                ui.painter().rect_filled(led, 0.0, theme::PHOSPHOR);
            } else {
                ui.painter().rect_stroke(
                    led,
                    0.0,
                    Stroke::new(1.0, theme::GREEN_MID),
                    StrokeKind::Inside,
                );
            }
            let said = format!("{name}: {}", if on { tr("Yes") } else { tr("No") });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
            theme::text(
                ui,
                name,
                theme::BODY,
                if on {
                    theme::PHOSPHOR
                } else {
                    theme::GREEN_MID
                },
            );
        }
    });
}

/// The strip of stat cells: state against the keyboard's own thresholds,
/// voltage, autonomy, signal.
fn cells(ui: &mut Ui, th: &Theme, snap: &Snapshot, kb: &KbReport, now: u64, per_row: usize) {
    let (state, state_level) = view::battery_state(&kb.battery);
    let volts = kb.battery.voltage.filter(|v| v.is_finite() && *v > 0.0);
    let rssi = kb.radio.rel_db();
    let items: [(&str, String, Level, u8); 4] = [
        (tr("State"), state.to_string(), state_level, 0),
        (
            tr("Voltage"),
            volts.map_or(view::DASH.into(), view::volts_text),
            volts.map_or(Level::Unknown, view::voltage_level),
            0,
        ),
        (
            tr("Remaining"),
            view::remaining_text(snap, now).unwrap_or_else(|| view::DASH.into()),
            Level::Unknown,
            0,
        ),
        (
            tr("Signal"),
            view::rssi_text(rssi),
            view::rssi_level(rssi),
            1 + view::rssi_bar_count(rssi),
        ),
    ];
    let avail = ui.available_width();
    let w = ((avail - (per_row - 1) as f32 * theme::GAP) / per_row as f32).floor();
    for row in items.chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP;
            for (label, value, level, bars) in row {
                let rect = th.cell(ui, w, label, value, *level);
                if *bars > 0 {
                    let at = Rect::from_min_size(
                        Pos2::new(rect.right() - 36.0, rect.top() + 7.0),
                        Vec2::new(28.0, 14.0),
                    );
                    theme::signal_bars(ui.painter(), at, bars - 1, theme::level_color(*level));
                }
            }
        });
    }
}

fn actions(
    ui: &mut Ui,
    th: &Theme,
    snap: &Snapshot,
    link: &Job,
    fnmode: &FnMode,
) -> Option<Request> {
    let mut request = None;
    th.panel(ui, tr("Actions"), 0.0, |ui| {
        ui.horizontal_wrapped(|ui| {
            shell::reconnect_button(ui, link);
            fn_button(ui, fnmode);
            if theme::action(ui, tr("Rename"), snap.mac().is_some())
                .on_hover_text(tr("Name of this keyboard on this computer"))
                .clicked()
            {
                request = Some(Request::Rename);
            }
        });
        shell::outcome(ui, th, &link.outcome());
        shell::outcome(ui, th, &fnmode.job.outcome());
    });
    request
}

/// Text of the Fn button: the current mode, `---` while it is not known.
pub fn fn_button_text(mode: Option<i32>) -> String {
    let now = mode
        .filter(|m| *m >= 0)
        .map_or(view::DASH.to_string(), crate::fn_toggle::mode_text);
    trf("Fn: {}", &[&now])
}

/// The button that toggles the function keys (polkit dialog of the daemon).
pub fn fn_button(ui: &mut Ui, fnmode: &FnMode) {
    let label = fn_button_text(fnmode.mode());
    if theme::action(ui, &label, !fnmode.job.busy())
        .on_hover_text(tr("Toggle the function keys"))
        .clicked()
    {
        let ctx = ui.ctx().clone();
        fnmode.toggle(move || ctx.request_repaint());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_number_is_never_an_invented_zero() {
        assert_eq!(hero_text(Some(86.4)), ("86".to_string(), "%"));
        assert_eq!(hero_text(Some(100.0)), ("100".to_string(), "%"));
        assert_eq!(hero_text(None), ("---".to_string(), ""));
        assert_eq!(hero_text(Some(f64::NAN)), ("---".to_string(), ""));
    }

    /// From 660 px (the narrowest wide layout) to a full HD screen: "100 %"
    /// fits in its panel, the figure in its own, and at the reference size
    /// the whole tab shows without scrolling.
    #[test]
    fn wide_layout_fits_from_the_narrowest_to_the_largest_window() {
        for (w, h, name) in [
            (theme::NARROW, 400.0, 20.0),
            (872.0, 552.0, 20.0),
            (872.0, 552.0, 60.0),
            (1180.0, 1040.0, 20.0),
            (640.0, 80.0, 40.0),
            (3000.0, 3000.0, 20.0),
        ] {
            let l = wide_layout(w, h, name);
            let left = ((w - theme::GAP) * BATTERY_SHARE).floor() - 24.0;
            let number = (3.0 * theme::ADVANCE + 0.42 * theme::ADVANCE + 0.08) * l.hero;
            assert!(
                l.hero >= theme::HERO_NARROW && l.hero <= HERO_MAX,
                "{w}x{h}: {l:?}"
            );
            assert!(
                number <= left + 0.5 || l.hero == theme::HERO_NARROW,
                "{w}x{h}: {number} > {left}"
            );
            assert!(
                l.hero * HERO_LINE + BATTERY_LINES <= l.top + 0.5,
                "{w}x{h}: {l:?}"
            );
            assert!(
                l.figure + KEYBOARD_LINES + name <= l.top + 0.5 || l.figure == 40.0,
                "{w}x{h}: {l:?}"
            );
            assert!(l.figure <= FIGURE_MAX_HEIGHT);
        }
        // 900 x 700: top row + cells + actions fit in the body.
        let l = wide_layout(872.0, 552.0, 20.0);
        let total = l.top + PANEL_CHROME + CELLS_HEIGHT + ACTIONS_HEIGHT + 2.0 * theme::GAP;
        assert!(total <= 552.0, "{total}");
        assert!(
            l.hero > theme::HERO,
            "a big number at the reference size: {l:?}"
        );
    }

    #[test]
    fn fn_button_says_the_mode_or_dashes() {
        assert_eq!(fn_button_text(Some(1)), "Fn: media keys first");
        assert_eq!(fn_button_text(Some(2)), "Fn: F1\u{2013}F12 first");
        assert_eq!(fn_button_text(None), "Fn: ---");
        assert_eq!(fn_button_text(Some(-1)), "Fn: ---", "hid_apple not loaded");
    }

    /// The keys of the figure stay inside its body, row after row, without
    /// overlap, whatever the size (also a degenerate one).
    #[test]
    fn figure_keys_tile_the_body() {
        for (w, h) in [(300.0, 90.0), (120.0, 40.0), (800.0, 132.0)] {
            let area = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(w, h));
            let keys = key_rects(area);
            assert_eq!(keys.len(), KEY_ROWS.iter().map(|r| r.len()).sum::<usize>());
            for (_, _, k) in &keys {
                assert!(
                    area.expand(0.01).contains_rect(*k),
                    "{k:?} outside {area:?}"
                );
                assert!(k.width() > 0.0 && k.height() > 0.0);
            }
            for pair in keys.windows(2) {
                let ((r0, _, a), (r1, _, b)) = (pair[0], pair[1]);
                if r0 == r1 {
                    assert!(a.right() <= b.left());
                    assert!((a.top() - b.top()).abs() < 1e-3);
                } else {
                    assert!(a.bottom() <= b.top());
                    // Every row ends on the right edge.
                    assert!((a.right() - area.right()).abs() < 0.01);
                }
            }
            let last = keys.last().unwrap().2;
            assert!((last.bottom() - area.bottom()).abs() < 0.01);
            assert!(keys.iter().any(|(r, c, _)| (*r, *c) == CAPS_KEY));
        }
    }
}
