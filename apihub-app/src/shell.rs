//! The device shell around the tabs: RobCo terminal header, Pip-Boy tab bar,
//! alert block and the permanent status bar; plus the keyboard navigation
//! (digits, arrows, Escape, F5) and the small blocks several tabs share.

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use akm_core::Snapshot;

use crate::actions::Outcome;
use crate::i18n::tr;
use crate::theme::{self, Theme};
use crate::view::{self, Feed, Level};

/// First line of the header, as on a RobCo terminal (not translated).
pub const TERMINAL_LINE: &str = "ROBCO INDUSTRIES (TM) TERMLINK PROTOCOL";

/// Points scrolled by one arrow key / one page key.
const SCROLL_LINE: f32 = 60.0;
const SCROLL_PAGE: f32 = 320.0;

const TAB_BAR_HEIGHT: f32 = 36.0;
// The tabs are clickable targets: at least 28 px tall.
const _: () = assert!(TAB_BAR_HEIGHT >= 28.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Stat,
    Radio,
    Keys,
    Data,
    Diag,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Stat, Tab::Radio, Tab::Keys, Tab::Data, Tab::Diag];

    /// Short capital label, the same in every language (Pip-Boy vocabulary).
    pub fn label(self) -> &'static str {
        match self {
            Tab::Stat => "STAT",
            Tab::Radio => "RADIO",
            Tab::Keys => "KEYS",
            Tab::Data => "DATA",
            Tab::Diag => "DIAG",
        }
    }

    /// What the tab holds, in words (hover text, screen reader).
    pub fn hint(self) -> &'static str {
        match self {
            Tab::Stat => tr("Batteries, link and actions"),
            Tab::Radio => tr("Bluetooth link and signal"),
            Tab::Keys => tr("Function keys and key mapping"),
            Tab::Data => tr("History, batteries, device and firmware"),
            Tab::Diag => tr("System Diagnostics"),
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// Tab of the digit key `n` (1 to 5).
    pub fn from_digit(n: usize) -> Option<Tab> {
        n.checked_sub(1).and_then(|i| Self::ALL.get(i).copied())
    }

    /// Next (`+1`) or previous (`-1`) tab, wrapping around.
    pub fn step(self, delta: i32) -> Tab {
        let n = Self::ALL.len() as i32;
        Self::ALL[(self.index() as i32 + delta).rem_euclid(n) as usize]
    }
}

/// What a key press asks of the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Nav {
    Go(Tab),
    Step(i32),
    Scroll(f32),
    /// Escape: leave the focused widget, else come back to STAT.
    Back,
    /// F5: reload what the current tab shows.
    Reload,
}

fn digit(key: egui::Key) -> Option<usize> {
    use egui::Key as K;
    [K::Num1, K::Num2, K::Num3, K::Num4, K::Num5]
        .iter()
        .position(|k| *k == key)
        .map(|i| i + 1)
}

/// Navigation of one key press. `typing`: a text field owns the keyboard
/// (nothing is taken from it); `focused`: a widget has the focus (arrows
/// then move the focus, as egui does). The physical key is looked at first
/// so that the digit row works without Shift on an AZERTY layout.
pub fn nav_for(
    key: egui::Key,
    physical: Option<egui::Key>,
    modifiers: egui::Modifiers,
    typing: bool,
    focused: bool,
) -> Option<Nav> {
    use egui::Key as K;
    if typing || modifiers.ctrl || modifiers.alt || modifiers.command {
        return None;
    }
    if let Some(tab) = physical
        .and_then(digit)
        .or_else(|| digit(key))
        .and_then(Tab::from_digit)
    {
        return Some(Nav::Go(tab));
    }
    match key {
        K::Escape => Some(Nav::Back),
        K::F5 => Some(Nav::Reload),
        _ if focused => None,
        K::ArrowLeft => Some(Nav::Step(-1)),
        K::ArrowRight => Some(Nav::Step(1)),
        K::ArrowUp => Some(Nav::Scroll(SCROLL_LINE)),
        K::ArrowDown => Some(Nav::Scroll(-SCROLL_LINE)),
        K::PageUp => Some(Nav::Scroll(SCROLL_PAGE)),
        K::PageDown => Some(Nav::Scroll(-SCROLL_PAGE)),
        _ => None,
    }
}

/// Navigation asked by the key presses of this frame.
pub fn navigation(ctx: &egui::Context) -> Vec<Nav> {
    let typing = ctx.wants_keyboard_input();
    let focused = ctx.memory(|m| m.focused().is_some());
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => nav_for(*key, *physical_key, *modifiers, typing, focused),
                _ => None,
            })
            .collect()
    })
}

/// Rows of a flow layout: items of `widths`, `sep` between two items of a
/// row, wrapped to `avail`. An item wider than `avail` gets its own row.
pub fn flow_rows(widths: &[f32], sep: f32, avail: f32) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        match rows.last_mut() {
            Some(row) if used + sep + w <= avail => {
                row.push(i);
                used += sep + w;
            }
            _ => {
                rows.push(vec![i]);
                used = *w;
            }
        }
    }
    rows
}

/// Local time of day of a unix timestamp (0 = never).
pub fn local_hms(unix: u64) -> Option<(u32, u32, u32)> {
    if unix == 0 {
        return None;
    }
    let t = libc::time_t::try_from(unix).ok()?;
    // SAFETY: `tm` is plain data, valid when zeroed; both pointers are valid
    // for the call and `localtime_r` is the thread-safe variant.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        tm
    };
    Some((
        u32::try_from(tm.tm_hour).ok()?,
        u32::try_from(tm.tm_min).ok()?,
        u32::try_from(tm.tm_sec).ok()?,
    ))
}

/// Header: terminal line and version, product name and link state, rule.
pub fn header(ui: &mut Ui, th: &Theme, snap: &Snapshot) {
    let avail = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(avail, theme::SMALL + theme::TITLE + 12.0),
        Sense::hover(),
    );
    let p = ui.painter();
    let line = p.text(
        rect.left_top(),
        Align2::LEFT_TOP,
        TERMINAL_LINE,
        theme::font(theme::SMALL),
        theme::GREEN_MID,
    );
    let version = format!("V{}", env!("CARGO_PKG_VERSION"));
    let vw = version.len() as f32 * theme::ADVANCE * theme::SMALL;
    if line.right() + theme::GAP + vw <= rect.right() {
        p.text(
            rect.right_top(),
            Align2::RIGHT_TOP,
            version,
            theme::font(theme::SMALL),
            theme::GREEN_MID,
        );
    }
    let y = rect.top() + theme::SMALL + 2.0;
    th.glow(
        p,
        Pos2::new(rect.left(), y),
        Align2::LEFT_TOP,
        &format!("> {}", theme::caps(tr("Apple Keyboard Monitor"))),
        theme::TITLE,
        theme::PHOSPHOR,
    );
    let (word, level) = if view::link_up(snap) {
        (tr("ONLINE"), Level::Good)
    } else if snap.keyboard.is_some() {
        (tr("OFFLINE"), Level::Bad)
    } else {
        (tr("NO DATA"), Level::Unknown)
    };
    let color = theme::level_color(level);
    let mid = y + theme::TITLE / 2.0;
    let text = th.glow(
        p,
        Pos2::new(rect.right(), mid),
        Align2::RIGHT_CENTER,
        word,
        theme::BODY,
        color,
    );
    let led = Rect::from_center_size(Pos2::new(text.left() - 12.0, mid + 1.0), Vec2::splat(10.0));
    if level == Level::Good {
        p.rect_filled(led, 0.0, color);
    } else {
        p.rect_stroke(led, 0.0, Stroke::new(1.5, color), StrokeKind::Inside);
    }
    theme::double_rule(p, rect.x_range(), rect.bottom() - 2.0, theme::PHOSPHOR);
}

/// Tab bar: five equal cells, the active one boxed and opening the rule.
/// Returns the tab clicked (or activated from the keyboard).
pub fn tab_bar(ui: &mut Ui, th: &Theme, current: Tab) -> Option<Tab> {
    let avail = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail, TAB_BAR_HEIGHT), Sense::hover());
    // The tabs stand over the (centred) body; the rule runs across the window.
    let tabs = Rect::from_center_size(
        rect.center(),
        Vec2::new(rect.width().min(theme::MAX_CONTENT), rect.height()),
    );
    let cell_w = tabs.width() / Tab::ALL.len() as f32;
    let mut clicked = None;
    let base = rect.bottom() - 0.5;
    let line = Stroke::new(1.0, theme::PHOSPHOR);
    let mut gap = rect.left()..=rect.left();
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(tabs.left() + i as f32 * cell_w, tabs.top()),
            Vec2::new(cell_w, TAB_BAR_HEIGHT),
        );
        let resp = ui
            .interact(cell, ui.id().with(("tab", i)), Sense::click())
            .on_hover_text(format!("{} · {}", i + 1, tab.hint()));
        let selected = tab == current;
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, tab.label())
        });
        if resp.clicked() {
            clicked = Some(tab);
        }
        let p = ui.painter();
        // The digit of the tab, then its label, centred as one group.
        let digit_w = theme::ADVANCE * theme::SMALL;
        let label_w = tab.label().len() as f32 * theme::ADVANCE * theme::TITLE;
        let group = digit_w + 4.0 + label_w;
        let x = cell.center().x - group / 2.0;
        let y = cell.center().y + 1.0;
        let ink = if selected || resp.hovered() || resp.has_focus() {
            theme::PHOSPHOR
        } else {
            theme::GREEN_MID
        };
        p.text(
            Pos2::new(x, y),
            Align2::LEFT_CENTER,
            i + 1,
            theme::font(theme::SMALL),
            theme::GREEN_MID,
        );
        let at = Pos2::new(x + digit_w + 4.0, y);
        if selected {
            th.glow(p, at, Align2::LEFT_CENTER, tab.label(), theme::TITLE, ink);
            let frame = Rect::from_min_max(
                Pos2::new((x - 8.0).max(cell.left() + 1.0), cell.top() + 3.0),
                Pos2::new((x + group + 8.0).min(cell.right() - 1.0), base),
            );
            let s = Stroke::new(1.5, theme::PHOSPHOR);
            p.line_segment([frame.left_bottom(), frame.left_top()], s);
            p.line_segment([frame.left_top(), frame.right_top()], s);
            p.line_segment([frame.right_top(), frame.right_bottom()], s);
            gap = frame.left()..=frame.right();
        } else {
            p.text(
                at,
                Align2::LEFT_CENTER,
                tab.label(),
                theme::font(theme::TITLE),
                ink,
            );
        }
        if resp.has_focus() {
            p.hline(
                x..=x + group,
                cell.bottom() - 6.0,
                Stroke::new(1.0, theme::GREEN_MID),
            );
        }
    }
    let p = ui.painter();
    p.hline(rect.left()..=*gap.start(), base, line);
    p.hline(*gap.end()..=rect.right(), base, line);
    clicked
}

/// Alert block: one line per alert, most urgent first; nothing when all is
/// well.
pub fn alerts(ui: &mut Ui, th: &Theme, snap: &Snapshot, feed: Feed) {
    let list = view::alerts(snap, feed);
    if list.is_empty() {
        return;
    }
    ui.add_space(theme::GAP);
    for a in &list {
        th.alert(ui, a.level, &a.text);
    }
}

/// Status bar: keyboard name, masked address, firmware, time of the last
/// read. Items flow onto as many rows as the width needs; nothing is cut.
pub fn status_bar(ui: &mut Ui, snap: &Snapshot) {
    let avail = ui.available_width();
    let small = theme::font(theme::SMALL);
    let fmt = |color| egui::TextFormat {
        font_id: small.clone(),
        color,
        ..Default::default()
    };
    let item = |label: &str, value: &str| {
        let mut job = egui::text::LayoutJob::default();
        if !label.is_empty() {
            job.append(&format!("{label} "), 0.0, fmt(theme::GREEN_MID));
        }
        job.append(&theme::glyphs(value), 0.0, fmt(theme::PHOSPHOR));
        job.wrap.max_width = avail;
        ui.fonts(|f| f.layout_job(job))
    };
    let name = snap.display_name().map_or(view::DASH.into(), theme::caps);
    let fw = snap
        .firmware()
        .and_then(|f| f.version.clone())
        .unwrap_or_else(|| view::DASH.into());
    let galleys = [
        item("", &name),
        item("", &view::mask_mac(snap.mac())),
        item("FW", &fw),
        item(tr("READ"), &view::clock_text(local_hms(snap.last_update))),
    ];
    let sep = 2.0 * theme::GAP + 1.0;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x).collect();
    let rows = flow_rows(&widths, sep, avail);
    let height: f32 = rows
        .iter()
        .map(|r| r.iter().map(|i| galleys[*i].size().y).fold(0.0, f32::max) + 2.0)
        .sum();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail, height + 8.0), Sense::hover());
    let p = ui.painter();
    p.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, theme::PHOSPHOR),
    );
    let mut y = rect.top() + 7.0;
    for row in &rows {
        let h = row.iter().map(|i| galleys[*i].size().y).fold(0.0, f32::max);
        let mut x = rect.left();
        for (n, i) in row.iter().enumerate() {
            if n > 0 {
                p.vline(
                    x + theme::GAP,
                    y + 3.0..=y + h - 3.0,
                    Stroke::new(1.0, theme::GREEN_FRAME),
                );
                x += sep;
            }
            p.galley(Pos2::new(x, y), galleys[*i].clone(), theme::PHOSPHOR);
            x += galleys[*i].size().x;
        }
        y += h + 2.0;
    }
    let said = galleys
        .iter()
        .map(|g| g.text())
        .collect::<Vec<_>>()
        .join(", ");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
}

/// Outcome of an action under its buttons: `> message` or `[!] error`.
pub fn outcome(ui: &mut Ui, th: &Theme, o: &Outcome) {
    match o {
        Some((true, m)) => {
            theme::text(ui, &format!("> {m}"), theme::BODY, theme::PHOSPHOR);
        }
        Some((false, m)) => th.alert(ui, Level::Bad, m),
        None => {}
    }
}

/// A shell command the user runs himself (the window never does): the
/// command in a box and a button copying it to the clipboard.
pub fn command_line(ui: &mut Ui, cmd: &str) {
    ui.horizontal_wrapped(|ui| {
        egui::Frame::new()
            .stroke(Stroke::new(1.0, theme::GREEN_FRAME))
            .fill(theme::BG)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                theme::text(ui, &format!("$ {cmd}"), theme::BODY, theme::PHOSPHOR);
            });
        if theme::action(ui, tr("Copy"), true)
            .on_hover_text(tr("Copy the command to the clipboard"))
            .clicked()
        {
            ui.ctx().copy_text(cmd.to_string());
        }
    });
}

/// The Reconnect button of STAT and RADIO.
pub fn reconnect_button(ui: &mut Ui, link: &crate::actions::Job) {
    if theme::action(ui, tr("Reconnect"), !link.busy())
        .on_hover_text(tr("Ask the daemon to page the keyboard now"))
        .clicked()
    {
        let ctx = ui.ctx().clone();
        link.start(
            crate::actions::CALL_TIMEOUT,
            move || ctx.request_repaint(),
            crate::actions::reconnect,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};

    #[test]
    fn tabs_are_five_short_capital_words() {
        let labels = Tab::ALL.map(Tab::label);
        assert_eq!(labels, ["STAT", "RADIO", "KEYS", "DATA", "DIAG"]);
        for (i, t) in Tab::ALL.into_iter().enumerate() {
            assert_eq!(t.index(), i);
            assert_eq!(Tab::from_digit(i + 1), Some(t));
            assert!(t.label().len() <= 5 && t.label() == t.label().to_uppercase());
            assert!(!t.hint().is_empty());
        }
        assert_eq!(Tab::from_digit(0), None);
        assert_eq!(Tab::from_digit(6), None);
        assert_eq!(Tab::Stat.step(-1), Tab::Diag);
        assert_eq!(Tab::Diag.step(1), Tab::Stat);
        assert_eq!(Tab::Radio.step(1), Tab::Keys);
    }

    /// The five tabs and their digits fit in the narrowest supported window.
    #[test]
    fn tab_bar_fits_in_420_px() {
        let avail = 420.0 - 2.0 * theme::GUTTER;
        let cell = avail / Tab::ALL.len() as f32;
        for t in Tab::ALL {
            let group = theme::ADVANCE * theme::SMALL
                + 4.0
                + t.label().len() as f32 * theme::ADVANCE * theme::TITLE;
            assert!(group + 16.0 <= cell, "{}: {group} in {cell}", t.label());
        }
        // The terminal line of the header too.
        let line = TERMINAL_LINE.len() as f32 * theme::ADVANCE * theme::SMALL;
        assert!(line <= avail, "{line} > {avail}");
    }

    #[test]
    fn keyboard_navigation() {
        let none = Modifiers::NONE;
        let nav = |k, typing, focused| nav_for(k, None, none, typing, focused);
        assert_eq!(nav(Key::Num1, false, false), Some(Nav::Go(Tab::Stat)));
        assert_eq!(nav(Key::Num5, false, true), Some(Nav::Go(Tab::Diag)));
        assert_eq!(nav(Key::Num6, false, false), None);
        // AZERTY: the logical key of the digit row is not a digit.
        assert_eq!(
            nav_for(Key::Quote, Some(Key::Num3), none, false, false),
            Some(Nav::Go(Tab::Keys))
        );
        // A text field owns the keyboard: nothing is taken from it.
        for k in [Key::Num2, Key::ArrowLeft, Key::Escape, Key::F5] {
            assert_eq!(nav(k, true, true), None);
        }
        assert_eq!(nav(Key::ArrowRight, false, false), Some(Nav::Step(1)));
        assert_eq!(nav(Key::ArrowLeft, false, false), Some(Nav::Step(-1)));
        assert_eq!(nav(Key::ArrowLeft, false, true), None, "moves the focus");
        assert_eq!(
            nav(Key::ArrowDown, false, false),
            Some(Nav::Scroll(-SCROLL_LINE))
        );
        assert_eq!(
            nav(Key::PageUp, false, false),
            Some(Nav::Scroll(SCROLL_PAGE))
        );
        assert_eq!(nav(Key::Escape, false, true), Some(Nav::Back));
        assert_eq!(nav(Key::F5, false, true), Some(Nav::Reload));
        assert_eq!(nav(Key::A, false, false), None);
        assert_eq!(
            nav_for(Key::Num1, None, Modifiers::CTRL, false, false),
            None
        );
        assert_eq!(nav_for(Key::Num1, None, Modifiers::ALT, false, false), None);
    }

    #[test]
    fn status_bar_items_wrap_instead_of_being_cut() {
        assert_eq!(
            flow_rows(&[100.0, 120.0, 60.0, 80.0], 17.0, 900.0),
            vec![vec![0, 1, 2, 3]]
        );
        // 420 px: the name alone, the three short items together.
        assert_eq!(
            flow_rows(&[390.0, 122.0, 65.0, 80.0], 17.0, 392.0),
            vec![vec![0], vec![1, 2, 3]]
        );
        assert_eq!(
            flow_rows(&[500.0, 500.0], 17.0, 100.0),
            vec![vec![0], vec![1]]
        );
        assert_eq!(flow_rows(&[], 17.0, 100.0), Vec::<Vec<usize>>::new());
        assert_eq!(
            flow_rows(&[10.0, 10.0], 17.0, f32::NAN),
            vec![vec![0], vec![1]]
        );
    }

    #[test]
    fn local_time_of_the_last_read() {
        assert_eq!(local_hms(0), None);
        let (h, m, s) = local_hms(1_790_000_000).unwrap();
        assert!(h < 24 && m < 60 && s < 61);
        assert_eq!(local_hms(u64::MAX), None);
    }
}
