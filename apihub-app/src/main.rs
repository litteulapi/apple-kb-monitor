mod fnmode_diag;
mod instance;
mod keyboard;
mod portal;
mod rename;
mod source;
mod tray;
mod view;

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use view::{Level, Palette};
use std::thread;
use std::time::Duration;

use akm_core::{Snapshot, Watch};
use eframe::egui;

/// Latest keyboard state, published by the acquisition actor.
type State = Arc<Watch>;

fn unix_now() -> u64 {
    akm_core::history::Clock::now(&akm_core::history::SystemClock)
}

// ── App ─────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Tab {
    Keyboard,
    Diag,
}

#[derive(Clone)]
struct DiagResult {
    label: String,
    ok: bool,
    detail: String,
}

struct ApiHubApp {
    state: State,
    tab: Tab,
    style_initialized: bool,
    diag_results: Arc<Mutex<Vec<DiagResult>>>,
    diag_running: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
    // Set by a second launch / D-Bus Activate / tray: bring the window to front
    tray_show_window: Arc<AtomicBool>,
    appearance: portal::Shared,
    applied: Option<portal::Appearance>,
    palette: Palette,
    // Battery history graph
    battery_history: Vec<(f64, f64)>,    // (timestamp, percentage)
    voltage_history: Vec<(f64, f64)>,    // (timestamp, voltage)
    // Rename field (#141)
    rename_buf: String,
    rename_loaded: Option<String>,
    rename_status: rename::Status,
}

impl ApiHubApp {
    /// Create the GUI app. Does NOT spawn the acquisition — it is owned by
    /// main() and shared through the `Watch`.
    fn new(
        cc: &eframe::CreationContext<'_>,
        state: State,
        tray_show_window: Arc<AtomicBool>,
        quit_flag: Arc<AtomicBool>,
    ) -> Self {
        // Load battery history from disk (once at startup)
        let entries = source::load_history();
        let battery_history: Vec<(f64, f64)> = entries
            .iter()
            .map(|e| (e.ts as f64, e.pct))
            .collect();
        let voltage_history: Vec<(f64, f64)> = entries
            .iter()
            .filter_map(|e| e.reliable_voltage().map(|v| (e.ts as f64, v)))
            .collect();

        Self {
            state,
            tab: Tab::Keyboard,
            style_initialized: false,
            diag_results: Arc::new(Mutex::new(Vec::new())),
            diag_running: Arc::new(AtomicBool::new(false)),
            quit_flag,
            tray_show_window,
            appearance: portal::spawn(cc.egui_ctx.clone()),
            applied: None,
            palette: Palette::new(cc.egui_ctx.style().visuals.dark_mode),
            battery_history,
            voltage_history,
            rename_buf: String::new(),
            rename_loaded: None,
            rename_status: Arc::new(Mutex::new(None)),
        }
    }
}

impl eframe::App for ApiHubApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tray "Quit" while the window is open: close it so main() can exit.
        if self.quit_flag.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // 16px minimum text size — once only
        if !self.style_initialized {
            let mut style = (*ctx.style()).clone();
            for (_text_style, font_id) in style.text_styles.iter_mut() {
                if font_id.size < 16.0 {
                    font_id.size = 16.0;
                }
            }
            style.spacing.item_spacing = egui::Vec2::new(8.0, 6.0);
            ctx.set_style(style);
            self.style_initialized = true;
        }

        // Follow the system (portal) light/dark scheme and accent, live.
        let wanted = *self.appearance.lock().unwrap_or_else(|e| e.into_inner());
        if self.applied != Some(wanted) {
            apply_appearance(ctx, &wanted);
            self.palette = Palette::new(ctx.style().visuals.dark_mode);
            self.applied = Some(wanted);
        }

        // Check if tray requested window show
        if self.tray_show_window.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // Next repaint in 2s — egui sleeps until then or until user interaction
        ctx.request_repaint_after(Duration::from_secs(2));

        let snap = self.state.get();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Keyboard, "Keyboard");
                ui.selectable_value(&mut self.tab, Tab::Diag, "Diag");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab {
                Tab::Keyboard => self.tab_keyboard(ui, &snap),
                Tab::Diag => self.tab_diag(ui),
            }
        });
    }
}

/// Light/dark + accent from the portal; no portal answer = toolkit default.
fn apply_appearance(ctx: &egui::Context, a: &portal::Appearance) {
    let mut visuals = match a.scheme {
        Some(portal::Scheme::Dark) => egui::Visuals::dark(),
        Some(portal::Scheme::Light) => egui::Visuals::light(),
        None => ctx.style().visuals.clone(),
    };
    if let Some([r, g, b]) = a.accent {
        let accent = view::accent_color([r, g, b]);
        visuals.selection.bg_fill = accent;
        visuals.selection.stroke.color = if visuals.dark_mode { egui::Color32::WHITE } else { egui::Color32::BLACK };
        visuals.hyperlink_color = accent;
    }
    visuals.window_rounding = egui::Rounding::same(8.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(4.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(4.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(4.0);
    visuals.widgets.active.rounding = egui::Rounding::same(4.0);
    ctx.set_visuals(visuals);
}

// ── Tabs ────────────────────────────────────────────────────────────────────

impl ApiHubApp {
    fn tint(&self, t: egui::RichText, l: Level) -> egui::RichText {
        match self.palette.color(l) {
            Some(c) => t.color(c),
            None => t,
        }
    }

    fn tab_keyboard(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        if let Some(ref err) = snap.kb_error {
            ui.add(egui::Label::new(egui::RichText::new(err.as_str()).size(16.0).color(self.palette.bad)).wrap());
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let Some(kb) = &snap.keyboard else {
                ui.label(egui::RichText::new("Waiting for keyboard data...").size(16.0));
                return;
            };
            let now = unix_now();
            // Two tiles per row only when each gets a usable width (#195).
            let wide = view::two_columns(ui.available_width());
            self.tile_row(ui, wide, |me, ui| me.battery_tile(ui, snap, kb, now), |me, ui| me.radio_tile(ui, snap, kb, now));
            ui.add_space(8.0);
            self.tile_row(ui, wide, |me, ui| me.device_tile(ui, snap, kb), |me, ui| me.firmware_tile(ui, kb));
            ui.add_space(8.0);
            self.draw_battery_history(ui);
        });
    }

    /// Two tiles side by side (equal widths) or stacked.
    fn tile_row(
        &mut self,
        ui: &mut egui::Ui,
        wide: bool,
        left: impl FnOnce(&mut Self, &mut egui::Ui),
        right: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        if wide {
            ui.columns(2, |cols| {
                if let [l, r] = cols {
                    tile(l, |ui| left(self, ui));
                    tile(r, |ui| right(self, ui));
                }
            });
        } else {
            tile(ui, |ui| left(self, ui));
            ui.add_space(8.0);
            tile(ui, |ui| right(self, ui));
        }
    }

    fn battery_tile(&mut self, ui: &mut egui::Ui, snap: &Snapshot, kb: &akm_core::report::KbReport, now: u64) {
        // Kernel / 0x47 first; 0xEA is not a percentage (#136). An estimate is
        // said to be one (#198).
        let src = view::pct_source(&kb.battery);
        let pct = src.value();
        ui.vertical_centered(|ui| {
            ui.label(self.tint(egui::RichText::new(view::pct_text(pct, 0)).size(28.0).strong(), view::battery_level(pct)));
            ui.label(egui::RichText::new(src.caption()).weak().size(14.0));
            ui.add(egui::ProgressBar::new(view::pct_fraction(pct)).text(view::pct_text(pct, 1)));
        });
        ui.add_space(4.0);
        kv_grid(ui, "bat_detail", |ui| {
            if let Some(v) = kb.battery.voltage.filter(|v| v.is_finite() && *v > 0.0) {
                // Measured: reports 0x46 / 0xFF, in mV (#139).
                key(ui, "Voltage");
                value(ui, self.tint(egui::RichText::new(view::volts_text(v)).strong().size(18.0), view::voltage_level(v)));
                ui.end_row();
            }
            if let Some(t) = view::estimate_text(&kb.battery) {
                // Charge estimated by the declared chemistry [hypothèse] (#178).
                key(ui, "Estimate");
                value(ui, egui::RichText::new(t).size(16.0));
                ui.end_row();
            }
            if let Some(t) = view::chemistry_text(&kb.battery) {
                key(ui, "Batteries");
                value(ui, egui::RichText::new(t).size(16.0));
                ui.end_row();
            }
            // The kernel % steps down only at reconnections (#179).
            key(ui, "Updated");
            value(ui, egui::RichText::new(view::age_text(snap.update_age_s(now))).size(16.0));
            ui.end_row();
            if let Some(rem) = view::remaining_text(snap, now) {
                key(ui, "Remaining");
                value(ui, egui::RichText::new(rem).size(16.0));
                ui.end_row();
            }
            key(ui, "LEDs");
            ui.horizontal(|ui| {
                let on = |b: bool| if b { Level::Good } else { Level::Unknown };
                let weak = |b: bool, t: egui::RichText| if b { t } else { t.weak() };
                ui.label(weak(snap.caps_lock, self.tint(egui::RichText::new("CAPS").size(16.0).strong(), on(snap.caps_lock))));
                ui.label(weak(snap.num_lock, self.tint(egui::RichText::new("NUM").size(16.0).strong(), on(snap.num_lock))));
            });
            ui.end_row();
        });
    }

    fn radio_tile(&mut self, ui: &mut egui::Ui, snap: &Snapshot, kb: &akm_core::report::KbReport, now: u64) {
        ui.label(egui::RichText::new("Radio").strong().size(18.0));
        ui.add_space(4.0);
        kv_grid(ui, "radio_detail", |ui| {
            // Relative BR/EDR value (dB to the ideal range), not dBm (#174).
            let rssi = kb.radio.rel_db();
            let lvl = view::rssi_level(rssi);
            key(ui, "Signal");
            ui.horizontal(|ui| {
                ui.label(self.tint(egui::RichText::new(view::rssi_text(rssi)).strong().size(18.0), lvl));
                let c = self.palette.color(lvl).unwrap_or_else(|| ui.visuals().text_color());
                signal_bars(ui, view::rssi_bar_count(rssi), c);
            });
            ui.end_row();
            if view::rssi_valid(rssi).is_some() {
                if let Some(age) = snap.rssi_age_s(now) {
                    key(ui, "Measured");
                    value(ui, egui::RichText::new(view::age_text(Some(age))).size(16.0));
                    ui.end_row();
                }
            }
            key(ui, "TX Power");
            value(ui, egui::RichText::new(view::tx_power_text(kb.radio.tx_power_dbm)).size(16.0));
            ui.end_row();
            key(ui, "Connected");
            let (txt, lvl) = if kb.bluetooth.connected { ("Yes", Level::Good) } else { ("No", Level::Bad) };
            value(ui, self.tint(egui::RichText::new(txt).strong().size(16.0), lvl));
            ui.end_row();
            if let Some(p) = view::paired_text(&kb.bluetooth) {
                key(ui, "Paired");
                value(ui, egui::RichText::new(p).size(16.0));
                ui.end_row();
            }
            if let Some(w) = view::wake_text(&kb.wake) {
                // Passive listening of input report 0x13.
                key(ui, "Last wake");
                value(ui, egui::RichText::new(w).size(16.0));
                ui.end_row();
            }
        });
    }

    fn device_tile(&mut self, ui: &mut egui::Ui, snap: &Snapshot, kb: &akm_core::report::KbReport) {
        ui.label(egui::RichText::new("Device").strong().size(18.0));
        ui.add_space(4.0);
        kv_grid(ui, "dev_left", |ui| {
            if let Some(ref model) = kb.device.model {
                key(ui, "Model");
                value(ui, egui::RichText::new(model).strong().size(16.0));
                ui.end_row();
            }
            if let Some(ref name) = kb.device.name {
                key(ui, "Own name");
                value(ui, egui::RichText::new(name).size(16.0));
                ui.end_row();
            }
            if let Some(ref mac) = kb.device.mac {
                key(ui, "MAC");
                value(ui, egui::RichText::new(mac).monospace().size(16.0));
                ui.end_row();
            }
            if let Some(ref driver) = kb.device.driver {
                key(ui, "Driver");
                value(ui, egui::RichText::new(driver.as_str()).size(16.0));
                ui.end_row();
            }
            if let Some(ref host) = kb.bluetooth.paired_host_addr {
                key(ui, "Paired host");
                value(ui, egui::RichText::new(host).monospace().size(16.0));
                ui.end_row();
            }
        });
        // The editor gets the full tile width, below the table (#195).
        if let Some(ref mac) = kb.device.mac {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Name").weak().size(16.0));
            self.rename_row(ui, mac, snap.display_name());
        }
    }

    fn firmware_tile(&mut self, ui: &mut egui::Ui, kb: &akm_core::report::KbReport) {
        ui.label(egui::RichText::new("Firmware").strong().size(18.0));
        ui.add_space(4.0);
        kv_grid(ui, "dev_right", |ui| {
            if let Some(ref chip) = kb.device.chip {
                key(ui, "Chip");
                value(ui, egui::RichText::new(chip.as_str()).size(16.0));
                ui.end_row();
            }
            if let Some(ref fw) = kb.firmware.version {
                key(ui, "Version (0x4F)");
                value(ui, egui::RichText::new(fw).strong().size(18.0));
                ui.end_row();
            }
            // Uninterpreted vendor reports (meaning not proven, #131/#132).
            for (id, hex) in &kb.raw {
                key(ui, &format!("{id} (raw)"));
                value(ui, egui::RichText::new(hex).monospace().size(16.0));
                ui.end_row();
            }
            if kb.incomplete {
                key(ui, "Read");
                value(ui, egui::RichText::new("incomplete (timeout)").size(16.0));
                ui.end_row();
            }
        });
    }

    /// Editable name of the keyboard: text field + Rename / Reset (BlueZ alias).
    fn rename_row(&mut self, ui: &mut egui::Ui, mac: &str, current: Option<&str>) {
        let current = current.unwrap_or_default().to_string();
        ui.vertical(|ui| {
            let mut submit: Option<String> = None;
            ui.horizontal(|ui| {
                // Leave room for the two buttons whatever the tile width (#195).
                let w = (ui.available_width() - 150.0).clamp(80.0, 260.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.rename_buf)
                        .desired_width(w)
                        .char_limit(akm_core::alias::MAX_CHARS)
                        .hint_text("Keyboard name"),
                );
                // Follow the daemon's name unless the user is typing.
                if self.rename_loaded.as_deref() != Some(current.as_str()) && !edit.has_focus() {
                    self.rename_buf = current.clone();
                    self.rename_loaded = Some(current.clone());
                }
                let changed = self.rename_buf.trim() != current;
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.add_enabled(changed, egui::Button::new("Rename")).clicked() || enter) && changed {
                    submit = Some(self.rename_buf.clone());
                }
                if ui.button("Reset").on_hover_text("Restore the keyboard's own name").clicked() {
                    submit = Some(String::new());
                }
            });
            if let Some(text) = submit {
                *self.rename_status.lock().unwrap_or_else(|e| e.into_inner()) = None;
                match rename::check(&text) {
                    Ok(name) => rename::submit(mac.to_string(), name, self.rename_status.clone(), ui.ctx().clone()),
                    Err(e) => *self.rename_status.lock().unwrap_or_else(|e| e.into_inner()) = Some((false, e)),
                }
            }
            if let Some((ok, msg)) = self.rename_status.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                let t = egui::RichText::new(msg).size(14.0);
                ui.add(egui::Label::new(if ok { t.weak() } else { t.color(self.palette.bad) }).wrap());
            }
        });
    }

    fn reload_history(&mut self) {
        let entries = source::load_history();
        self.battery_history = entries.iter().map(|e| (e.ts as f64, e.pct)).collect();
        self.voltage_history = entries.iter().filter_map(|e| e.reliable_voltage().map(|v| (e.ts as f64, v))).collect();
    }

    /// Draw battery + voltage history chart (last 24 h) using egui painter.
    fn draw_battery_history(&mut self, ui: &mut egui::Ui) {
        tile(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Battery History (24 h)").strong().size(18.0));
                if ui.button(egui::RichText::new("Refresh").size(14.0)).clicked() {
                    self.reload_history();
                }
            });

            let cutoff = unix_now() as f64 - 24.0 * 3600.0;
            // Unusable points (NaN, out of range) are never drawn (#198).
            let batt_data = view::chart_points(&self.battery_history, cutoff, true);
            let volt_data = view::chart_points(&self.voltage_history, cutoff, false);

            if batt_data.len() < 2 {
                let msg = if self.battery_history.is_empty() { "No history data yet." } else { "Not enough data points in the last 24 h." };
                ui.label(egui::RichText::new(msg).weak().size(16.0));
                return;
            }

            let (t_min, t_max) = match (batt_data.first(), batt_data.last()) {
                (Some(a), Some(b)) => (a.0, b.0),
                _ => return,
            };
            ui.label(egui::RichText::new(format!(
                "{} points over {:.1} h",
                batt_data.len(),
                (t_max - t_min) / 3600.0
            )).weak().size(14.0));
            ui.add_space(4.0);

            let (response, painter) = ui.allocate_painter(egui::Vec2::new(ui.available_width(), 160.0), egui::Sense::hover());
            let rect = response.rect;
            let vis = ui.visuals().clone();
            painter.rect_filled(rect, 4.0, vis.extreme_bg_color);
            // Room on the left for the % labels, on top for the legend.
            let plot_rect = egui::Rect::from_min_max(
                egui::Pos2::new(rect.min.x + 36.0, rect.min.y + 22.0),
                egui::Pos2::new(rect.max.x - 8.0, rect.max.y - 8.0),
            );
            if plot_rect.width() < 10.0 || plot_rect.height() < 10.0 {
                return;
            }
            let painter = painter.with_clip_rect(rect);
            let t_range = (t_max - t_min).max(1.0);
            let x_of = |ts: f64| plot_rect.min.x + ((ts - t_min) / t_range) as f32 * plot_rect.width();
            let y_of = |frac: f64| plot_rect.max.y - (frac.clamp(0.0, 1.0) as f32) * plot_rect.height();

            for level in [0.0, 25.0, 50.0, 75.0, 100.0] {
                let y = y_of(level / 100.0);
                painter.line_segment(
                    [egui::Pos2::new(plot_rect.min.x, y), egui::Pos2::new(plot_rect.max.x, y)],
                    egui::Stroke::new(0.5, vis.widgets.noninteractive.bg_stroke.color),
                );
                painter.text(
                    egui::Pos2::new(plot_rect.min.x - 4.0, y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{level:.0}%"),
                    egui::FontId::proportional(10.0),
                    vis.weak_text_color(),
                );
            }

            let batt_color = self.palette.good;
            let line: Vec<egui::Pos2> = batt_data.iter().map(|&(t, p)| egui::Pos2::new(x_of(t), y_of(p / 100.0))).collect();
            painter.add(egui::Shape::line(line, egui::Stroke::new(2.0, batt_color)));

            let volt_color = self.palette.info;
            let mut legend = vec![(batt_color, "Battery %".to_string())];
            if volt_data.len() >= 2 {
                let v_min = volt_data.iter().map(|p| p.1).fold(f64::MAX, f64::min);
                let v_max = volt_data.iter().map(|p| p.1).fold(f64::MIN, f64::max);
                let v_range = (v_max - v_min).max(0.1);
                let (v_lo, v_hi) = (v_min - v_range * 0.05, v_max + v_range * 0.05);
                let line: Vec<egui::Pos2> = volt_data
                    .iter()
                    .filter(|p| p.0 >= t_min)
                    .map(|&(t, v)| egui::Pos2::new(x_of(t), y_of((v - v_lo) / (v_hi - v_lo))))
                    .collect();
                painter.add(egui::Shape::line(line, egui::Stroke::new(1.5, volt_color)));
                legend.push((volt_color, format!("Voltage {v_min:.2}\u{2013}{v_max:.2} V (own scale)")));
            }

            // Legend: swatch then its text, laid out right to left.
            let mut x = rect.max.x - 8.0;
            for (color, text) in legend.iter().rev() {
                let galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(11.0), vis.weak_text_color());
                x -= galley.size().x;
                let y = rect.min.y + 5.0;
                painter.galley(egui::Pos2::new(x, y), galley.clone(), vis.weak_text_color());
                x -= 12.0;
                painter.rect_filled(egui::Rect::from_min_size(egui::Pos2::new(x, y + 2.0), egui::Vec2::splat(8.0)), 1.0, *color);
                x -= 14.0;
            }
        });
    }

    fn run_diagnostics(&mut self) {
        // Guard against concurrent runs
        if self.diag_running.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Ok(mut r) = self.diag_results.lock() {
            r.clear();
        }

        let results = self.diag_results.clone();
        let running = self.diag_running.clone();

        /// Clears the "running" flag even if the diagnostics thread panics.
        struct RunningGuard(Arc<AtomicBool>);
        impl Drop for RunningGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Relaxed);
            }
        }

        thread::spawn(move || {
            let _guard = RunningGuard(running);
            let mut out: Vec<DiagResult> = Vec::new();

            let checks: Vec<(&str, Vec<String>, &str)> = vec![
                ("apple-kb-monitord", vec!["--version".into()], "Monitor daemon binary"),
                ("keyd", vec!["-v".into()], "Key remapping daemon"),
                ("bluetoothctl", vec!["--version".into()], "BlueZ CLI"),
            ];

            for (bin, args, desc) in &checks {
                let result = Command::new(bin)
                    .args(args.iter().map(|s| s.as_str()))
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output();
                match result {
                    Ok(o) if o.status.success() => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let first = stdout.lines().next().unwrap_or("OK").trim();
                        out.push(DiagResult {
                            label: desc.to_string(), ok: true,
                            detail: format!("{}: {}", bin, if first.is_empty() { "OK" } else { first }),
                        });
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        if err.contains("Usage") || err.contains("usage") {
                            out.push(DiagResult {
                                label: desc.to_string(), ok: true,
                                detail: format!("{}: installed", bin),
                            });
                        } else {
                            out.push(DiagResult {
                                label: desc.to_string(), ok: false,
                                detail: format!("{}: exit {}", bin, o.status.code().unwrap_or(-1)),
                            });
                        }
                    }
                    Err(_) => {
                        out.push(DiagResult {
                            label: desc.to_string(), ok: false,
                            detail: format!("{}: NOT FOUND", bin),
                        });
                    }
                }
            }

            // Daemon: owner of the keyboard, reached over the session bus
            let active = Command::new("systemctl")
                .args(["--user", "is-active", "apple-kb-monitord.service"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            out.push(DiagResult {
                label: "apple-kb-monitord.service".into(), ok: active,
                detail: if active { "active (running)".into() } else { "inactive / not found".into() },
            });
            let on_bus = zbus::blocking::Connection::session()
                .map(|c| apple_kb_monitord::client::daemon_present(&c))
                .unwrap_or(false);
            out.push(DiagResult {
                label: "D-Bus com.agenceapi.AppleKbMonitor1".into(), ok: on_bus,
                detail: if on_bus { "daemon reachable (this window is a client)".into() }
                        else { "daemon absent: this app reads the keyboard itself".into() },
            });

            // Apple keyboard hidraw node: present AND readable by this user
            // (udev rule uses TAG+="uaccess", no group membership needed).
            let (hid_ok, hid_detail) = match keyboard::find_apple_hidraw() {
                None => (false, "no Apple hidraw device found (keyboard off or not paired?)".to_string()),
                Some(path) => match std::fs::File::open(&path) {
                    Ok(_) => (true, format!("{}: readable", path)),
                    Err(e) => (false, format!("{}: {} — check the udev uaccess rule", path, e)),
                },
            };
            out.push(DiagResult { label: "hidraw readable".into(), ok: hid_ok, detail: hid_detail });

            // keyd config
            let keyd_conf = std::path::Path::new("/etc/keyd/apple-keyboard.conf").exists();
            let keyd_running = Command::new("systemctl")
                .args(["is-active", "--quiet", "keyd.service"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            out.push(DiagResult {
                label: "keyd config".into(), ok: keyd_conf && keyd_running,
                detail: match (keyd_conf, keyd_running) {
                    (true, true) => "/etc/keyd/apple-keyboard.conf, keyd.service active".into(),
                    (true, false) => "/etc/keyd/apple-keyboard.conf present but keyd.service is NOT active".into(),
                    (false, _) => "NOT FOUND".into(),
                },
            });

            // udev rules
            let udev_ok = std::path::Path::new("/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules").exists();
            out.push(DiagResult {
                label: "udev rules".into(), ok: udev_ok,
                detail: if udev_ok { "70-apple-kb-hidraw.rules installed".into() } else { "NOT FOUND".into() },
            });

            // hid_apple fnmode: applied value (sysfs) vs configured (modprobe.d)
            let (fn_ok, fn_detail) = fnmode_diag::diagnose();
            out.push(DiagResult { label: "hid_apple fnmode".into(), ok: fn_ok, detail: fn_detail });

            // rssi-helper caps
            let rssi_ok = std::path::Path::new("/usr/lib/apple-kb-monitor/rssi-helper").exists();
            out.push(DiagResult {
                label: "RSSI helper".into(), ok: rssi_ok,
                detail: if rssi_ok { "rssi-helper installed (needs CAP_NET_ADMIN)".into() } else { "NOT FOUND".into() },
            });

            // Store results (the guard clears the running flag on drop)
            if let Ok(mut r) = results.lock() {
                *r = out;
            }
        });
    }

    fn tab_diag(&mut self, ui: &mut egui::Ui) {
        let is_running = self.diag_running.load(std::sync::atomic::Ordering::Relaxed);

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("System Diagnostics").strong().size(18.0));
            if is_running {
                ui.label(egui::RichText::new("Running...").size(16.0).color(self.palette.warn));
            } else if ui.button(egui::RichText::new("Run Full Check").size(16.0).strong()).clicked() {
                self.run_diagnostics();
            }
        });
        ui.separator();

        let results = self.diag_results.lock().map(|r| r.clone()).unwrap_or_default();

        if results.is_empty() {
            if is_running {
                ui.label(egui::RichText::new("Diagnostics in progress...").weak().size(16.0));
            } else {
                ui.label(egui::RichText::new("Press 'Run Full Check' to scan all components.").weak().size(16.0));
            }
            return;
        }

        let total = results.len();
        let ok_count = results.iter().filter(|r| r.ok).count();
        let fail_count = total - ok_count;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{}/{} passed", ok_count, total)).strong().size(18.0)
                .color(if fail_count == 0 { self.palette.good } else { self.palette.warn }));
            if fail_count > 0 {
                ui.label(egui::RichText::new(format!("  {} issues", fail_count)).size(16.0)
                    .color(self.palette.bad));
            }
        });

        ui.add_space(8.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("diag").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                for r in &results {
                    let (icon, c) = if r.ok { ("OK", self.palette.good) } else { ("FAIL", self.palette.bad) };
                    ui.label(egui::RichText::new(icon).size(16.0).strong().color(c));
                    ui.label(egui::RichText::new(&r.label).strong().size(16.0));
                    // Long details wrap instead of running off the window (#195).
                    ui.add(egui::Label::new(egui::RichText::new(&r.detail).weak().size(16.0)).wrap());
                    ui.end_row();
                }
            });
        });
    }
}

/// A framed tile that fills the width it is given (equal columns, #195).
fn tile(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// Two-column key/value table whose values never widen the tile.
fn kv_grid(ui: &mut egui::Ui, id: &str, rows: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([16.0, 8.0]).show(ui, rows);
}

fn key(ui: &mut egui::Ui, k: &str) {
    ui.label(egui::RichText::new(k).weak().size(16.0));
}

/// A value cell: truncated with "…" when too long, full text on hover (#195).
fn value(ui: &mut egui::Ui, t: egui::RichText) {
    let full = t.text().to_string();
    let r = ui.add(egui::Label::new(t).truncate());
    if full.chars().count() > 20 {
        r.on_hover_text(full);
    }
}

/// Four signal bars drawn with the painter (the block glyphs are missing
/// from egui's fonts, #198).
fn signal_bars(ui: &mut egui::Ui, lit: u8, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(26.0, 16.0), egui::Sense::hover());
    let off = ui.visuals().widgets.noninteractive.bg_stroke.color;
    for i in 0..4u8 {
        let h = 4.0 + 3.5 * f32::from(i);
        let x = rect.min.x + f32::from(i) * 6.5;
        let r = egui::Rect::from_min_max(egui::Pos2::new(x, rect.max.y - h), egui::Pos2::new(x + 4.5, rect.max.y));
        ui.painter().rect_filled(r, 1.0, if i < lit { color } else { off });
    }
}

// ── Entrypoint ──────────────────────────────────────────────────────────────

/// Open the window and block until it is closed. Returns false when it
/// could not be opened (no display / GPU). Called **once** per process:
/// winit does not support a second event loop run reliably (#226).
fn open_window(state: &State, raise: &Arc<AtomicBool>, quit_flag: &Arc<AtomicBool>, open: &Arc<AtomicBool>) -> bool {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Apple Keyboard Monitor")
            .with_app_id(instance::APP_ID)
            .with_inner_size([720.0, 600.0])
            .with_min_inner_size([500.0, 400.0]),
        vsync: true,
        // Return to main() on close, which then ends the process (#226).
        run_and_return: true,
        ..Default::default()
    };
    let (st, sw, qf) = (state.clone(), raise.clone(), quit_flag.clone());
    raise.store(false, Ordering::Relaxed);
    open.store(true, Ordering::Relaxed);
    let r = eframe::run_native(instance::APP_ID, options, Box::new(move |cc| Ok(Box::new(ApiHubApp::new(cc, st, sw, qf)))));
    open.store(false, Ordering::Relaxed);
    if let Err(ref e) = r {
        eprintln!("[apihub] cannot open window: {}", e);
    }
    r.is_ok()
}

fn main() {
    // One window = one process (#226): `apihub-app` or D-Bus
    // `org.freedesktop.Application` Activate opens the window; a second launch
    // raises it and exits; closing the window ends the process, which frees
    // the D-Bus name and any legacy tray icon. Nothing ever reopens a window
    // by itself. The tray belongs to the daemon; the legacy tray of this
    // process only lives while the window is open, when the daemon has none.
    let window_open = Arc::new(AtomicBool::new(false));
    let raise = Arc::new(AtomicBool::new(false));
    let activate = {
        let raise = raise.clone();
        move |token: Option<String>| {
            // The window is (being) opened by this process: just raise it.
            // The token cannot be used any more (the window is already
            // mapped) and `set_var` from this D-Bus thread would be
            // undefined behaviour (#197).
            let _ = token;
            raise.store(true, Ordering::Relaxed);
        }
    };
    let conn = match instance::claim(activate) {
        instance::Claim::Existing => {
            eprintln!("[apihub] already running: window raised");
            return;
        }
        instance::Claim::Unreachable => std::process::exit(1),
        instance::Claim::Primary(c) => Some(c),
        instance::Claim::NoBus => None,
    };

    let state: State = Arc::new(Watch::new());
    let quit_flag = Arc::new(AtomicBool::new(false));
    let src = source::spawn(state.clone());

    if !instance::daemon_tray_present() {
        eprintln!("[apihub] no daemon tray: legacy tray while the window is open");
        // Clicks only raise the open window; nothing is queued for later (#226).
        let (tx, _rx_dropped) = mpsc::channel();
        tray::spawn(state.clone(), raise.clone(), quit_flag.clone(), tx);
    }

    let shown = open_window(&state, &raise, &quit_flag, &window_open);
    eprintln!("[apihub] window closed: exiting");
    // Free the name first so that a new launch becomes the window at once.
    drop(conn);
    src.stop();
    std::process::exit(if shown { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_shows_na_without_sources() {
        let t = Snapshot::default().tooltip_text();
        assert!(t.contains("n/a"));
        assert!(!t.contains("0%"));
    }

    #[test]
    fn diag_results_survive_a_panicking_writer() {
        let r: Arc<Mutex<Vec<DiagResult>>> = Arc::new(Mutex::new(Vec::new()));
        let r2 = r.clone();
        let _ = thread::spawn(move || {
            let _g = r2.lock().unwrap();
            panic!("boom");
        })
        .join();
        assert!(r.lock().is_err());
        r.clear_poison();
        assert!(r.lock().is_ok());
    }
}
