//! Pip-Boy theme of the window (docs/DA-PIPBOY.md): the canonical palette,
//! the VT323 font, the egui style and the drawing primitives. Every colour
//! and every size of the window comes from this module.
//!
//! The CRT effects (scanlines, vignette, glow) are STATIC shapes drawn with
//! the egui painter: nothing here animates or asks for a repaint, so the
//! window keeps its event-driven cadence (#231).

use std::borrow::Cow;
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

use crate::view::Level;

// ── Palette (PIPBOY-THEME.md, canonical: no other colour anywhere) ──────────

/// Phosphor green: main text, active elements, glow.
pub const PHOSPHOR: Color32 = Color32::from_rgb(0x15, 0xFF, 0x00);
/// Medium green: secondary text and labels (still readable on the CRT).
pub const GREEN_MID: Color32 = Color32::from_rgb(0x0A, 0xCC, 0x00);
/// Dark green: decoration only (unlit keys, leaders), never a text to read.
pub const GREEN_DIM: Color32 = Color32::from_rgb(0x0A, 0x9A, 0x00);
/// Frame green: borders, rules, unlit gauge segments.
pub const GREEN_FRAME: Color32 = Color32::from_rgb(0x0A, 0x7A, 0x00);
/// CRT background.
pub const BG: Color32 = Color32::from_rgb(0x02, 0x12, 0x06);
/// Lighter CRT background: panels, gradient.
pub const BG_PANEL: Color32 = Color32::from_rgb(0x04, 0x18, 0x0A);
/// Amber: warning.
pub const AMBER: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x41);
/// Red: error, alert.
pub const RED: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x3C);

// ── Sizes and grid ──────────────────────────────────────────────────────────

/// Smallest text of the window.
pub const SMALL: f32 = 18.0;
pub const BODY: f32 = 20.0;
pub const TITLE: f32 = 24.0;
/// Key values of the stat cells.
pub const VALUE: f32 = 30.0;
/// The battery percentage, wide and narrow windows.
pub const HERO: f32 = 132.0;
pub const HERO_NARROW: f32 = 104.0;
/// Space between two blocks.
pub const GAP: f32 = 8.0;
/// Distance from the window edge to the bezel line, then to the content.
pub const BEZEL_INSET: f32 = 5.0;
pub const GUTTER: f32 = 14.0;
/// Smallest height of anything that can be clicked (>= 28 px).
pub const TARGET: f32 = 30.0;
/// Height of one key/value row.
pub const ROW: f32 = 24.0;
/// Content narrower than this is laid out in one column.
pub const NARROW: f32 = 640.0;
/// The tabs never stretch wider than this: beyond, they stay centred.
pub const MAX_CONTENT: f32 = 1180.0;
/// Distance between two scanlines.
pub const SCANLINE_PITCH: f32 = 3.0;

// Guarantees of the grid, checked at compile time: clickable targets of at
// least 28 px, no text under 16 px, sizes in order.
const _: () = assert!(TARGET >= 28.0);
const _: () = assert!(SMALL >= 16.0 && SMALL <= BODY && BODY <= TITLE && TITLE <= VALUE);
const _: () = assert!(HERO_NARROW < HERO);

const SEGMENT_WIDTH: f32 = 12.0;
const SEGMENT_GAP: f32 = 4.0;
const SEGMENT_SKEW: f32 = 5.0;
const SEGMENTS_MAX: usize = 48;
/// VT323 is monospaced: every glyph advances by this fraction of the size.
pub const ADVANCE: f32 = 0.4;

const VT323: &[u8] = include_bytes!("../assets/fonts/VT323-Regular.ttf");
pub const FONT_NAME: &str = "VT323";

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// `c` with its opacity multiplied by `a`.
pub fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// Colour of a semantic level; unknown is the neutral secondary green.
pub fn level_color(l: Level) -> Color32 {
    match l {
        Level::Good => PHOSPHOR,
        Level::Warn => AMBER,
        Level::Bad => RED,
        Level::Unknown => GREEN_MID,
    }
}

/// Colour of a value: as [`level_color`], but a value without level is
/// plain phosphor text.
pub fn value_color(l: Level) -> Color32 {
    match l {
        Level::Unknown => PHOSPHOR,
        l => level_color(l),
    }
}

/// VT323 first, egui's own fonts as fallback for the glyphs it lacks.
pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(VT323)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, FONT_NAME.to_owned());
    }
    fonts
}

/// The egui style of the terminal: square corners, phosphor strokes.
pub fn style(base: &egui::Style) -> egui::Style {
    use egui::TextStyle as T;
    let mut s = base.clone();
    s.text_styles = [
        (T::Small, font(SMALL)),
        (T::Body, font(BODY)),
        (T::Button, font(BODY)),
        (T::Heading, font(TITLE)),
        (T::Monospace, FontId::monospace(BODY)),
    ]
    .into();
    s.spacing.item_spacing = Vec2::new(GAP, 6.0);
    s.spacing.button_padding = Vec2::new(10.0, 4.0);
    s.spacing.interact_size = Vec2::new(TARGET, TARGET);
    s.spacing.scroll = egui::style::ScrollStyle::solid();
    s.spacing.scroll.bar_width = 6.0;
    s.spacing.scroll.bar_inner_margin = 6.0;
    s.spacing.scroll.bar_outer_margin = 0.0;
    s.spacing.scroll.foreground_color = true;
    let square = egui::CornerRadius::ZERO;
    let v = &mut s.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG_PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = BG_PANEL;
    v.code_bg_color = BG_PANEL;
    v.window_stroke = Stroke::new(1.0, PHOSPHOR);
    v.window_corner_radius = square;
    v.menu_corner_radius = square;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.selection.bg_fill = GREEN_FRAME;
    v.selection.stroke = Stroke::new(1.0, PHOSPHOR);
    v.hyperlink_color = PHOSPHOR;
    v.text_cursor.stroke = Stroke::new(2.0, PHOSPHOR);
    v.disabled_alpha = 0.6;
    let w = &mut v.widgets;
    for (state, border, fill) in [
        (&mut w.noninteractive, GREEN_FRAME, BG_PANEL),
        (&mut w.inactive, GREEN_MID, BG_PANEL),
        (&mut w.hovered, PHOSPHOR, fade(GREEN_FRAME, 0.45)),
        (&mut w.active, PHOSPHOR, GREEN_FRAME),
        (&mut w.open, PHOSPHOR, BG_PANEL),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, border);
        state.fg_stroke = Stroke::new(1.0, PHOSPHOR);
        state.corner_radius = square;
        state.expansion = 0.0;
    }
    w.active.bg_stroke.width = 2.0;
    s
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.set_style(style(&ctx.style()));
}

// ── Pure helpers (unit-tested) ──────────────────────────────────────────────

/// Text as the terminal font can show it. Neither VT323 nor egui's
/// fallback fonts have the narrow no-break space of the French typography
/// or the arrows: the first becomes a no-break space, `→` becomes `>` and
/// `↔` becomes `<>`. The tilde of VT323 is a small raised mark that reads
/// as a stray letter: before a number ("~41 days") it becomes `≈`.
pub fn glyphs(s: &str) -> Cow<'_, str> {
    if !s.contains(['\u{202f}', '\u{2192}', '\u{2194}', '~']) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s.chars().peekable();
    while let Some(c) = rest.next() {
        match c {
            '\u{202f}' => out.push('\u{a0}'),
            '\u{2192}' => out.push('>'),
            '\u{2194}' => out.push_str("<>"),
            '~' if rest.peek().is_some_and(|n| n.is_ascii_digit() || *n == ' ') => {
                out.push('\u{2248}')
            }
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Upper-case label text (titles, keys of the key/value rows, buttons).
pub fn caps(s: &str) -> String {
    glyphs(s).to_uppercase()
}

/// Number of blocks of a segmented gauge `width` wide.
pub fn segment_count(width: f32) -> usize {
    if !width.is_finite() {
        return 1;
    }
    let n = ((width - SEGMENT_SKEW + SEGMENT_GAP) / (SEGMENT_WIDTH + SEGMENT_GAP)).floor();
    (n.max(1.0) as usize).min(SEGMENTS_MAX)
}

/// Lit blocks of a gauge of `n` blocks: nothing for 0 or unknown, at least
/// one block as soon as there is something, all of them only when full.
pub fn lit_segments(frac: f32, n: usize) -> usize {
    if !frac.is_finite() || frac <= 0.0 || n == 0 {
        return 0;
    }
    if frac >= 1.0 {
        return n;
    }
    ((frac * n as f32).round() as usize).clamp(1, n.saturating_sub(1).max(1))
}

/// Largest size of `sizes` (sorted from the largest) at which `chars`
/// monospaced glyphs fit in `avail`; the smallest one when none fits (the
/// caller then wraps).
pub fn fit_size(chars: usize, avail: f32, sizes: &[f32]) -> f32 {
    sizes
        .iter()
        .copied()
        .find(|s| chars as f32 * ADVANCE * s <= avail)
        .or(sizes.last().copied())
        .unwrap_or(BODY)
}

/// A key and its value share one row when both fit with a leader between.
pub fn kv_inline(key_width: f32, value_width: f32, avail: f32) -> bool {
    key_width + value_width + 3.0 * GAP <= avail
}

/// Y of every scanline of a screen `height` tall.
pub fn scanlines(height: f32) -> impl Iterator<Item = f32> {
    let n = if height.is_finite() && height > 0.0 {
        (height / SCANLINE_PITCH).ceil() as usize
    } else {
        0
    };
    (0..n.min(4096)).map(|i| i as f32 * SCANLINE_PITCH)
}

// ── Theme: the drawing primitives ───────────────────────────────────────────

/// Offsets (in glow radii) and opacity of the halo copies of a glowing text.
const HALO: [(f32, f32, f32); 8] = [
    (-1.0, 0.0, 0.14),
    (1.0, 0.0, 0.14),
    (0.0, -1.0, 0.14),
    (0.0, 1.0, 0.14),
    (-2.0, 0.0, 0.06),
    (2.0, 0.0, 0.06),
    (0.0, -2.0, 0.06),
    (0.0, 2.0, 0.06),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Scanlines, vignette and glow (`[ui] crt_effects`, default on).
    pub crt: bool,
}

impl Theme {
    /// Text with a phosphor halo (plain text when the CRT effects are off).
    /// Returns the rectangle of the text.
    pub fn glow(
        &self,
        p: &egui::Painter,
        pos: Pos2,
        anchor: Align2,
        text: &str,
        size: f32,
        color: Color32,
    ) -> Rect {
        let galley = p.layout_no_wrap(glyphs(text).into_owned(), font(size), color);
        let rect = anchor.anchor_size(pos, galley.size());
        if self.crt {
            let r = (size / 44.0).clamp(0.75, 2.0);
            for (dx, dy, a) in HALO {
                p.galley_with_override_text_color(
                    rect.min + Vec2::new(dx * r, dy * r),
                    galley.clone(),
                    fade(color, a),
                );
            }
        }
        p.galley(rect.min, galley, color);
        rect
    }

    /// A glowing one-line value laid out as a widget.
    pub fn glow_label(&self, ui: &mut Ui, text: &str, size: f32, color: Color32) -> Response {
        let galley = ui
            .painter()
            .layout_no_wrap(glyphs(text).into_owned(), font(size), color);
        let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::hover());
        self.glow(ui.painter(), rect.min, Align2::LEFT_TOP, text, size, color);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
        resp
    }

    /// Background of the device: CRT gradient and the bezel frame. Drawn on
    /// the background layer, before any panel.
    pub fn backdrop(&self, ctx: &egui::Context) {
        let p = ctx.layer_painter(egui::LayerId::background());
        let screen = ctx.screen_rect();
        p.rect_filled(screen, 0.0, BG);
        let inner = screen.shrink(BEZEL_INSET);
        if inner.width() < 8.0 || inner.height() < 8.0 {
            return;
        }
        // Tube gradient: lighter in the middle, CRT background at the edges.
        let mut mesh = egui::Mesh::default();
        let ys = [inner.top(), inner.center().y, inner.bottom()];
        for (i, y) in ys.iter().enumerate() {
            let c = if i == 1 { BG_PANEL } else { BG };
            mesh.colored_vertex(Pos2::new(inner.left(), *y), c);
            mesh.colored_vertex(Pos2::new(inner.right(), *y), c);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 2, 3);
        mesh.add_triangle(2, 3, 4);
        mesh.add_triangle(3, 4, 5);
        p.add(mesh);
        // Bezel: a frame line and four phosphor corner brackets.
        p.rect_stroke(
            inner,
            0.0,
            Stroke::new(1.0, GREEN_FRAME),
            StrokeKind::Inside,
        );
        let arm = 14.0_f32.min(inner.width() / 4.0).min(inner.height() / 4.0);
        let s = Stroke::new(2.0, PHOSPHOR);
        for (corner, dx, dy) in [
            (inner.left_top(), 1.0, 1.0),
            (inner.right_top(), -1.0, 1.0),
            (inner.left_bottom(), 1.0, -1.0),
            (inner.right_bottom(), -1.0, -1.0),
        ] {
            p.line_segment([corner, corner + Vec2::new(dx * arm, 0.0)], s);
            p.line_segment([corner, corner + Vec2::new(0.0, dy * arm)], s);
        }
    }

    /// Scanlines and vignette over the whole screen (top-most layer). Static.
    pub fn overlay(&self, ctx: &egui::Context) {
        if !self.crt {
            return;
        }
        let p = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            egui::Id::new("crt-overlay"),
        ));
        let screen = ctx.screen_rect();
        let line = fade(BG, 0.26);
        let mut mesh = egui::Mesh::default();
        for y in scanlines(screen.height()) {
            let top = screen.top() + y;
            mesh.add_colored_rect(
                Rect::from_min_max(
                    Pos2::new(screen.left(), top),
                    Pos2::new(screen.right(), top + 1.0),
                ),
                line,
            );
        }
        p.add(mesh);
        // Vignette: the four edges fade into the CRT background.
        let depth = 40.0_f32
            .min(screen.width() / 3.0)
            .min(screen.height() / 3.0);
        let (dark, clear) = (fade(BG, 0.42), fade(BG, 0.0));
        let inner = screen.shrink(depth);
        let mut v = egui::Mesh::default();
        for (a, b, c, d) in [
            (
                screen.left_top(),
                screen.right_top(),
                inner.right_top(),
                inner.left_top(),
            ),
            (
                screen.left_bottom(),
                screen.right_bottom(),
                inner.right_bottom(),
                inner.left_bottom(),
            ),
            (
                screen.left_top(),
                screen.left_bottom(),
                inner.left_bottom(),
                inner.left_top(),
            ),
            (
                screen.right_top(),
                screen.right_bottom(),
                inner.right_bottom(),
                inner.right_top(),
            ),
        ] {
            let i = v.vertices.len() as u32;
            v.colored_vertex(a, dark);
            v.colored_vertex(b, dark);
            v.colored_vertex(c, clear);
            v.colored_vertex(d, clear);
            v.add_triangle(i, i + 1, i + 2);
            v.add_triangle(i, i + 2, i + 3);
        }
        p.add(v);
    }

    /// A box whose title sits in its top border (`┌─ TITLE ───┐`), filling
    /// the width it is given; at least `min_height` tall.
    pub fn panel<R>(
        &self,
        ui: &mut Ui,
        title: &str,
        min_height: f32,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let bg = ui.painter().add(egui::Shape::Noop);
        let frame = egui::Frame::new().inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 24,
            bottom: 10,
        });
        let out = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(min_height);
            add(ui)
        });
        let rect = out.response.rect;
        let boxr =
            Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 10.0), rect.max).shrink(0.5);
        let p = ui.painter();
        p.set(bg, egui::Shape::rect_filled(boxr, 0.0, fade(BG_PANEL, 0.8)));
        let title_galley = p.layout_no_wrap(caps(title), font(BODY), PHOSPHOR);
        let x0 = boxr.left() + 10.0;
        let x1 = (x0 + title_galley.size().x + 12.0).min(boxr.right());
        let s = Stroke::new(1.0, GREEN_FRAME);
        p.hline(boxr.left()..=x0, boxr.top(), s);
        p.hline(x1..=boxr.right(), boxr.top(), s);
        p.hline(boxr.x_range(), boxr.bottom(), s);
        p.vline(boxr.left(), boxr.y_range(), s);
        p.vline(boxr.right(), boxr.y_range(), s);
        let clip = p.with_clip_rect(rect.intersect(p.clip_rect()));
        clip.galley(
            Pos2::new(x0 + 6.0, boxr.top() - title_galley.size().y / 2.0),
            title_galley,
            PHOSPHOR,
        );
        out.inner
    }

    /// One `KEY ........ value` row; the value goes under the key, wrapped,
    /// when both do not fit on one line. Nothing is ever truncated.
    pub fn kv(&self, ui: &mut Ui, key: &str, value: &str, level: Level) {
        let avail = ui.available_width();
        let color = if value == crate::view::DASH {
            GREEN_MID
        } else {
            value_color(level)
        };
        let value = glyphs(value).into_owned();
        let p = ui.painter();
        let key_g = p.layout_no_wrap(caps(key), font(BODY), GREEN_MID);
        let one = p.layout_no_wrap(value.clone(), font(BODY), color);
        let said = format!("{key}: {value}");
        if kv_inline(key_g.size().x, one.size().x, avail) {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail, ROW), Sense::hover());
            let p = ui.painter();
            let y = rect.center().y;
            let kx = rect.left() + key_g.size().x;
            let vx = rect.right() - one.size().x;
            p.galley(
                Pos2::new(rect.left(), y - key_g.size().y / 2.0),
                key_g,
                GREEN_MID,
            );
            p.galley(Pos2::new(vx, y - one.size().y / 2.0), one, color);
            leader(p, kx + GAP, vx - GAP, y + 5.0);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
        } else {
            let indent = 2.0 * GAP;
            let wrapped = p.layout(value, font(BODY), color, (avail - indent).max(40.0));
            let h = ROW + wrapped.size().y + 2.0;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail, h), Sense::hover());
            let p = ui.painter();
            p.galley(
                Pos2::new(rect.left(), rect.top() + (ROW - key_g.size().y) / 2.0),
                key_g,
                GREEN_MID,
            );
            p.galley(
                Pos2::new(rect.left() + indent, rect.top() + ROW),
                wrapped,
                color,
            );
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
        }
    }

    /// A boxed cell of the stat strip: small label, big glowing value sized
    /// to the cell.
    pub fn cell(&self, ui: &mut Ui, width: f32, label: &str, value: &str, level: Level) -> Rect {
        let pad = GAP;
        let inner = (width - 2.0 * pad).max(20.0);
        let chars = value.chars().count();
        let size = fit_size(chars, inner, &[VALUE, 26.0, 22.0, BODY]);
        let color = if value == crate::view::DASH {
            GREEN_MID
        } else {
            value_color(level)
        };
        let wrapped = ui
            .painter()
            .layout(glyphs(value).into_owned(), font(size), color, inner);
        let one_line = wrapped.rows.len() <= 1;
        let h = pad + SMALL + 2.0 + wrapped.size().y.max(VALUE) + pad;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
        let p = ui.painter();
        p.rect(
            rect.shrink(0.5),
            0.0,
            fade(BG_PANEL, 0.8),
            Stroke::new(1.0, GREEN_FRAME),
            StrokeKind::Inside,
        );
        p.text(
            rect.left_top() + Vec2::new(pad, pad - 2.0),
            Align2::LEFT_TOP,
            caps(label),
            font(SMALL),
            GREEN_MID,
        );
        let at = Pos2::new(rect.left() + pad, rect.top() + pad + SMALL + 2.0);
        if one_line {
            let y = at.y + (wrapped.size().y.max(VALUE) - wrapped.size().y) / 2.0;
            self.glow(p, Pos2::new(at.x, y), Align2::LEFT_TOP, value, size, color);
        } else {
            p.galley(at, wrapped, color);
        }
        let said = format!("{label}: {value}");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
        rect
    }

    /// One line of the alert block: a coloured bar, `[!]` and the text.
    pub fn alert(&self, ui: &mut Ui, level: Level, text_: &str) {
        let color = value_color(level);
        let avail = ui.available_width();
        let indent = 4.0 * ADVANCE * BODY + GAP;
        let g = ui.painter().layout(
            glyphs(text_).into_owned(),
            font(BODY),
            color,
            (avail - indent).max(40.0),
        );
        let h = g.size().y.max(ROW);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail, h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, h)), 0.0, color);
        self.glow(
            p,
            Pos2::new(rect.left() + 8.0, rect.top() + (ROW - BODY) / 2.0),
            Align2::LEFT_TOP,
            "[!]",
            BODY,
            color,
        );
        p.galley(
            Pos2::new(rect.left() + indent, rect.top() + (ROW - BODY) / 2.0),
            g,
            color,
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text_));
    }
}

/// Dotted leader between a key and its value.
fn leader(p: &egui::Painter, from: f32, to: f32, y: f32) {
    let mut x = from;
    while x + 2.0 <= to {
        p.rect_filled(
            Rect::from_min_size(Pos2::new(x, y), Vec2::splat(2.0)),
            0.0,
            GREEN_FRAME,
        );
        x += 7.0;
    }
}

/// Wrapping text.
pub fn text(ui: &mut Ui, s: &str, size: f32, color: Color32) -> Response {
    ui.add(egui::Label::new(RichText::new(glyphs(s)).size(size).color(color)).wrap())
}

/// A terminal button: `[ LABEL ]`, at least [`TARGET`] tall.
pub fn action(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(format!("[ {} ]", caps(label))).size(BODY))
            .min_size(Vec2::new(TARGET, TARGET)),
    )
}

/// One position of a selector, inverse video when selected.
pub fn choice(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let label = caps(label);
    let g = ui
        .painter()
        .layout_no_wrap(label.clone(), font(BODY), PHOSPHOR);
    let size = Vec2::new(g.size().x + 20.0, TARGET);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let (fill, ink, border) = if selected {
        (PHOSPHOR, BG, PHOSPHOR)
    } else if resp.hovered() || resp.has_focus() {
        (fade(GREEN_FRAME, 0.45), PHOSPHOR, PHOSPHOR)
    } else {
        (BG_PANEL, PHOSPHOR, GREEN_FRAME)
    };
    p.rect(
        rect.shrink(0.5),
        0.0,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    if resp.has_focus() {
        p.rect_stroke(
            rect.shrink(3.0),
            0.0,
            Stroke::new(1.0, ink),
            StrokeKind::Inside,
        );
    }
    p.galley_with_override_text_color(rect.center() - g.size() / 2.0, g, ink);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, &label)
    });
    resp
}

/// Horizontal rule with end ticks (`├────┤`).
pub fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    let s = Stroke::new(1.0, GREEN_FRAME);
    let p = ui.painter();
    p.hline(rect.x_range(), rect.center().y, s);
    p.vline(rect.left() + 0.5, rect.y_range(), s);
    p.vline(rect.right() - 0.5, rect.y_range(), s);
}

/// Double rule (`═════`) across `x`, centred on `y`.
pub fn double_rule(p: &egui::Painter, x: egui::Rangef, y: f32, color: Color32) {
    let s = Stroke::new(1.0, color);
    p.hline(x, y - 1.5, s);
    p.hline(x, y + 1.5, s);
}

/// Segmented gauge (`▰▰▰▱▱`) as wide as the space left, `lit` giving the
/// colour of each lit block.
pub fn segments(ui: &mut Ui, height: f32, n: usize, lit: impl Fn(usize) -> Option<Color32>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    paint_segments(ui.painter(), rect, n, lit);
}

pub fn paint_segments(
    p: &egui::Painter,
    rect: Rect,
    n: usize,
    lit: impl Fn(usize) -> Option<Color32>,
) {
    if n == 0 || rect.width() < SEGMENT_SKEW + 2.0 {
        return;
    }
    let step = (rect.width() - SEGMENT_SKEW + SEGMENT_GAP) / n as f32;
    let w = (step - SEGMENT_GAP).max(1.0);
    for i in 0..n {
        let x = rect.left() + i as f32 * step;
        let pts = vec![
            Pos2::new(x + SEGMENT_SKEW, rect.top()),
            Pos2::new(x + SEGMENT_SKEW + w, rect.top()),
            Pos2::new(x + w, rect.bottom()),
            Pos2::new(x, rect.bottom()),
        ];
        match lit(i) {
            Some(c) => p.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE)),
            None => p.add(egui::Shape::closed_line(pts, Stroke::new(1.0, GREEN_FRAME))),
        };
    }
}

/// Four signal bars of rising height, `lit` of them in `color`.
pub fn signal_bars(p: &egui::Painter, rect: Rect, lit: u8, color: Color32) {
    let step = rect.width() / 4.0;
    for i in 0..4u8 {
        let h = rect.height() * (0.25 + 0.25 * f32::from(i));
        let x = rect.left() + f32::from(i) * step;
        let r = Rect::from_min_max(
            Pos2::new(x, rect.bottom() - h),
            Pos2::new(x + step - 3.0, rect.bottom()),
        );
        if i < lit {
            p.rect_filled(r, 0.0, color);
        } else {
            p.rect_stroke(r, 0.0, Stroke::new(1.0, GREEN_FRAME), StrokeKind::Inside);
        }
    }
}

/// Two blocks side by side (`left_frac` of the width for the first one).
pub fn split(
    ui: &mut Ui,
    left_frac: f32,
    left: impl FnOnce(&mut Ui, f32),
    right: impl FnOnce(&mut Ui, f32),
) {
    let avail = ui.available_width();
    let lw = ((avail - GAP) * left_frac).floor();
    let rw = (avail - GAP - lw).floor();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let col = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(Vec2::new(lw, 0.0), col, |ui| {
            ui.set_width(lw);
            left(ui, lw);
        });
        ui.allocate_ui_with_layout(Vec2::new(rw, 0.0), col, |ui| {
            ui.set_width(rw);
            right(ui, rw);
        });
    });
}

/// Vertical scroll area of the tab `name` (each tab keeps its own scroll
/// position); applies the scroll asked from the keyboard (see
/// [`request_scroll`]).
pub fn scroll_body(ui: &mut Ui, name: &str, add: impl FnOnce(&mut Ui)) {
    let id = scroll_id();
    let dy: f32 = ui.ctx().data_mut(|d| d.remove_temp(id).unwrap_or(0.0));
    egui::ScrollArea::vertical()
        .id_salt(name)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if dy != 0.0 {
                ui.scroll_with_delta(Vec2::new(0.0, dy));
            }
            add(ui);
        });
}

fn scroll_id() -> egui::Id {
    egui::Id::new("key-scroll")
}

/// Scroll the tab body by `dy` points at the next layout (arrow keys).
pub fn request_scroll(ctx: &egui::Context, dy: f32) {
    ctx.data_mut(|d| d.insert_temp(scroll_id(), dy));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color32, b: Color32) -> f32 {
        fn lum(c: Color32) -> f32 {
            let f = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn palette_is_the_canonical_one() {
        let hex = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        assert_eq!(
            [PHOSPHOR, GREEN_MID, GREEN_DIM, GREEN_FRAME].map(hex),
            ["#15ff00", "#0acc00", "#0a9a00", "#0a7a00"]
        );
        assert_eq!(
            [BG, BG_PANEL, AMBER, RED].map(hex),
            ["#021206", "#04180a", "#ffb641", "#ff5a3c"]
        );
    }

    /// Every colour used for a text to read is above WCAG AA (4.5) on
    /// both backgrounds; the dark green is not a text colour.
    #[test]
    fn text_colours_are_readable_on_the_crt() {
        for bg in [BG, BG_PANEL] {
            for ink in [PHOSPHOR, GREEN_MID, AMBER, RED] {
                assert!(contrast(ink, bg) >= 4.5, "{ink:?} on {bg:?}");
            }
        }
        for l in [Level::Good, Level::Warn, Level::Bad, Level::Unknown] {
            assert_ne!(level_color(l), GREEN_DIM);
            assert_ne!(level_color(l), GREEN_FRAME);
            assert_ne!(value_color(l), GREEN_DIM);
        }
        assert_eq!(value_color(Level::Unknown), PHOSPHOR);
        // Inverse video of a selected choice.
        assert!(contrast(BG, PHOSPHOR) >= 7.0);
    }

    #[test]
    fn sizes_respect_the_minimums() {
        let s = style(&egui::Style::default());
        assert!(s.spacing.interact_size.y >= 28.0);
        for f in s.text_styles.values() {
            assert!(f.size >= SMALL);
        }
        assert_eq!(s.visuals.panel_fill, BG);
    }

    #[test]
    fn gauge_blocks() {
        assert_eq!(lit_segments(0.0, 20), 0);
        assert_eq!(lit_segments(f32::NAN, 20), 0);
        assert_eq!(lit_segments(-1.0, 20), 0);
        assert_eq!(lit_segments(0.01, 20), 1, "something left: one block");
        assert_eq!(lit_segments(0.5, 20), 10);
        assert_eq!(lit_segments(0.86, 20), 17);
        assert_eq!(lit_segments(0.99, 20), 19, "full only at 100 %");
        assert_eq!(lit_segments(1.0, 20), 20);
        assert_eq!(lit_segments(7.0, 20), 20);
        assert_eq!(lit_segments(0.5, 0), 0);
        assert_eq!(lit_segments(0.5, 1), 1);
        assert_eq!(segment_count(0.0), 1);
        assert_eq!(segment_count(f32::NAN), 1);
        assert_eq!(segment_count(-50.0), 1);
        assert_eq!(segment_count(100_000.0), SEGMENTS_MAX);
        let n = segment_count(360.0);
        assert!((18..=24).contains(&n), "{n}");
        for w in [1.0, 37.0, 360.0, 850.0, 1900.0] {
            let n = segment_count(w) as f32;
            assert!(
                n == 1.0 || n * (SEGMENT_WIDTH + SEGMENT_GAP) - SEGMENT_GAP + SEGMENT_SKEW <= w
            );
        }
    }

    #[test]
    fn values_shrink_to_their_cell_then_wrap() {
        let sizes = [VALUE, 26.0, 22.0, BODY];
        assert_eq!(fit_size(4, 160.0, &sizes), VALUE);
        assert_eq!(fit_size(14, 160.0, &sizes), 26.0);
        assert_eq!(fit_size(19, 160.0, &sizes), BODY);
        assert_eq!(fit_size(60, 160.0, &sizes), BODY, "too long: wrapped");
        assert_eq!(fit_size(3, 160.0, &[]), BODY);
    }

    #[test]
    fn key_value_rows_never_truncate() {
        assert!(kv_inline(80.0, 200.0, 360.0));
        assert!(!kv_inline(80.0, 280.0, 360.0), "value goes under the key");
        assert!(!kv_inline(10.0, 10.0, f32::NAN));
    }

    #[test]
    fn scanlines_cover_the_screen_and_stay_bounded() {
        assert_eq!(scanlines(0.0).count(), 0);
        assert_eq!(scanlines(f32::NAN).count(), 0);
        assert_eq!(scanlines(-3.0).count(), 0);
        let ys: Vec<f32> = scanlines(700.0).collect();
        assert_eq!(ys.len(), 234);
        assert!(ys.windows(2).all(|w| w[1] - w[0] == SCANLINE_PITCH));
        assert!(*ys.last().unwrap() < 700.0);
        assert_eq!(scanlines(1e9).count(), 4096);
    }

    #[test]
    fn text_is_made_showable_by_the_terminal_font() {
        assert_eq!(glyphs("2,91\u{202f}V"), "2,91\u{a0}V");
        assert_eq!(glyphs("→ application"), "> application");
        assert_eq!(glyphs("Linux PC (Cmd↔Alt)"), "Linux PC (Cmd<>Alt)");
        assert_eq!(glyphs("~41 jours"), "\u{2248}41 jours");
        assert_eq!(glyphs("~ 3 h"), "\u{2248} 3 h");
        assert_eq!(
            glyphs("~/.config/x"),
            "~/.config/x",
            "a path keeps its tilde"
        );
        assert!(matches!(glyphs("plain"), Cow::Borrowed(_)));
        assert_eq!(caps("état des piles"), "ÉTAT DES PILES");
        assert_eq!(caps("il y a 2\u{202f}h"), "IL Y A 2\u{a0}H");
    }

    /// VT323 itself has the French accents, the percent sign and the
    /// typographic signs of the catalogue.
    #[test]
    fn font_covers_french_and_falls_back_for_the_rest() {
        let only = {
            let mut f = egui::FontDefinitions::empty();
            f.font_data.insert(
                FONT_NAME.to_owned(),
                Arc::new(egui::FontData::from_static(VT323)),
            );
            f.families
                .insert(egui::FontFamily::Proportional, vec![FONT_NAME.to_owned()]);
            f.families
                .insert(egui::FontFamily::Monospace, vec![FONT_NAME.to_owned()]);
            f
        };
        let has = |defs: egui::FontDefinitions, s: &str| {
            let ctx = egui::Context::default();
            ctx.set_fonts(defs);
            let mut ok = false;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                ok = ctx.fonts(|f| f.has_glyphs(&font(BODY), s));
            });
            ok
        };
        let french = "é è à ç ù É È À Ç Ù ê î ô û ë ï % 0123456789 « » ’ … — – · ≈ −\u{a0}";
        assert!(has(only.clone(), french));
        assert!(has(
            only.clone(),
            "[ RECONNECTER ] > STAT RADIO KEYS DATA DIAG"
        ));
        // The installed definitions: VT323 first, egui's fonts kept behind.
        let defs = font_definitions();
        assert_eq!(defs.families[&egui::FontFamily::Proportional][0], FONT_NAME);
        assert_eq!(defs.families[&egui::FontFamily::Monospace][0], FONT_NAME);
        assert!(defs.families[&egui::FontFamily::Proportional].len() > 1);
        // No installed font has the arrows, the box characters, the block
        // gauges or the narrow no-break space: the window draws its frames
        // and gauges with the painter and `glyphs` replaces the two others.
        for missing in ["→", "─", "│", "┌", "═", "▰", "▱", "█", "\u{202f}"] {
            assert!(
                !has(defs.clone(), missing),
                "{missing:?} can be a glyph now"
            );
        }
        assert!(has(defs.clone(), &glyphs("→ 2,91\u{202f}V")));
        // Every text of the catalogue, once passed through `glyphs`, is
        // covered by the installed fonts.
        for (en, fr) in crate::i18n::parse_po(include_str!("../i18n/fr.po")) {
            for s in [en, fr] {
                assert!(has(defs.clone(), &glyphs(&s)), "glyph missing in {s:?}");
            }
        }
    }
}
