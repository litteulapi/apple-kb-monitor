//! Small egui building blocks shared by the tabs (extracted from `main.rs`, #62).

use eframe::egui;

/// A framed tile that fills the width it is given (equal columns, #195).
pub fn tile(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// Two-column key/value table whose values never widen the tile.
pub fn kv_grid(ui: &mut egui::Ui, id: &str, rows: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, rows);
}

pub fn key(ui: &mut egui::Ui, k: &str) {
    ui.label(egui::RichText::new(k).weak().size(16.0));
}

/// A value cell: truncated with "…" when too long, full text on hover (#195).
pub fn value(ui: &mut egui::Ui, t: egui::RichText) {
    let full = t.text().to_string();
    let r = ui.add(egui::Label::new(t).truncate());
    if full.chars().count() > 20 {
        r.on_hover_text(full);
    }
}

/// Four signal bars drawn with the painter (the block glyphs are missing
/// from egui's fonts, #198).
pub fn signal_bars(ui: &mut egui::Ui, lit: u8, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(26.0, 16.0), egui::Sense::hover());
    let off = ui.visuals().widgets.noninteractive.bg_stroke.color;
    for i in 0..4u8 {
        let h = 4.0 + 3.5 * f32::from(i);
        let x = rect.min.x + f32::from(i) * 6.5;
        let r = egui::Rect::from_min_max(
            egui::Pos2::new(x, rect.max.y - h),
            egui::Pos2::new(x + 4.5, rect.max.y),
        );
        ui.painter()
            .rect_filled(r, 1.0, if i < lit { color } else { off });
    }
}
