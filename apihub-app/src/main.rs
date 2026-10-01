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
            .filter_map(|e| e.voltage.map(|v| (e.ts as f64, v)))
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
            ui.label(egui::RichText::new(err.as_str()).size(16.0).color(self.palette.bad));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
        match &snap.keyboard {
            None => {
                ui.label(egui::RichText::new("Waiting for keyboard data...").size(16.0));
            }
            Some(kb) => {
                // Kernel / 0x47 first; 0xEA is not a percentage (#136).
                let pct: Option<f64> = kb.battery_pct();

                // Top row: battery tile + radio tile side by side
                ui.columns(2, |cols| {
                    // LEFT: Battery tile
                    cols[0].group(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(self.tint(
                                egui::RichText::new(view::pct_text(pct, 0)).size(28.0).strong(),
                                view::battery_level(pct)));
                            // Battery type subtitle under hero percentage
                            if let Some(v) = kb.battery.voltage {
                                ui.label(egui::RichText::new(format!("{} (estimation)", keyboard::detect_battery_type(v)))
                                    .weak().size(16.0));
                            }
                            ui.add(egui::ProgressBar::new(view::pct_fraction(pct))
                                .text(view::pct_text(pct, 1)));
                        });
                        ui.add_space(4.0);
                        egui::Grid::new("bat_detail").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            if let Some(v) = kb.battery.voltage {
                                // [hypothèse] adc * 3.3 / 1023: an estimate, not a measurement.
                                ui.label(egui::RichText::new("Voltage (est.)").weak().size(16.0));
                                ui.label(self.tint(
                                    egui::RichText::new(format!("≈ {:.2} V", v)).strong().size(18.0),
                                    view::voltage_level(v)));
                                ui.end_row();
                            }
                            if let Some(adc) = kb.battery.adc_raw {
                                ui.label(egui::RichText::new("0xF5 (raw)").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{}", adc)).size(16.0));
                                ui.end_row();
                            }
                            if let Some(ref rem) = snap.remaining_display {
                                ui.label(egui::RichText::new("Remaining").weak().size(16.0));
                                ui.label(egui::RichText::new(rem).size(16.0));
                                ui.end_row();
                            }
                            // LED state
                            ui.label(egui::RichText::new("LEDs").weak().size(16.0));
                            ui.horizontal(|ui| {
                                let on = |b: bool| if b { Level::Good } else { Level::Unknown };
                                let weak = |b: bool, t: egui::RichText| if b { t } else { t.weak() };
                                ui.label(weak(snap.caps_lock, self.tint(egui::RichText::new("CAPS").size(16.0).strong(), on(snap.caps_lock))));
                                ui.label(weak(snap.num_lock, self.tint(egui::RichText::new("NUM").size(16.0).strong(), on(snap.num_lock))));
                            });
                            ui.end_row();
                        });
                    });

                    // RIGHT: Radio tile
                    cols[1].group(|ui| {
                        ui.label(egui::RichText::new("Radio").strong().size(18.0));
                        ui.add_space(4.0);
                        egui::Grid::new("radio_detail").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            let rssi = kb.radio.rssi_dbm;
                            ui.label(egui::RichText::new("RSSI").weak().size(16.0));
                            let lvl = view::rssi_level(rssi);
                            ui.horizontal(|ui| {
                                ui.label(self.tint(egui::RichText::new(view::rssi_text(rssi)).strong().size(18.0), lvl));
                                ui.label(self.tint(egui::RichText::new(view::rssi_bars(rssi)).size(18.0), lvl));
                                if view::rssi_valid(rssi).is_some() {
                                    if let Some(age) = snap.rssi_age_s(unix_now()) {
                                        ui.label(egui::RichText::new(format!("({:.0}s ago)", age)).weak().size(12.0));
                                    }
                                }
                            });
                            ui.end_row();
                            ui.label(egui::RichText::new("TX Power").weak().size(16.0));
                            ui.label(egui::RichText::new(view::tx_power_text(kb.radio.tx_power_dbm)).size(16.0));
                            ui.end_row();
                            ui.label(egui::RichText::new("Connected").weak().size(16.0));
                            let (txt, lvl) = if kb.bluetooth.connected { ("Yes", Level::Good) } else { ("No", Level::Bad) };
                            ui.label(self.tint(egui::RichText::new(txt).strong().size(16.0), lvl));
                            ui.end_row();

                            ui.label(egui::RichText::new("Paired").weak().size(16.0));
                            ui.label(egui::RichText::new(if kb.bluetooth.paired { "Yes" } else { "No" }).size(16.0));
                            ui.end_row();
                        });
                    });
                });

                ui.add_space(8.0);

                // Bottom: Device info in two columns
                ui.columns(2, |cols| {
                    // LEFT: Identity
                    cols[0].group(|ui| {
                        ui.label(egui::RichText::new("Device").strong().size(18.0));
                        ui.add_space(4.0);
                        egui::Grid::new("dev_left").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            if let Some(ref model) = kb.device.model {
                                ui.label(egui::RichText::new("Model").weak().size(16.0));
                                ui.label(egui::RichText::new(model).strong().size(16.0));
                                ui.end_row();
                            }
                            if let Some(ref name) = kb.device.name {
                                ui.label(egui::RichText::new("Own name").weak().size(16.0));
                                ui.label(egui::RichText::new(name).size(16.0));
                                ui.end_row();
                            }
                            if let Some(ref mac) = kb.device.mac {
                                ui.label(egui::RichText::new("Name").weak().size(16.0));
                                self.rename_row(ui, mac, snap.display_name());
                                ui.end_row();
                            }
                            if let Some(ref mac) = kb.device.mac {
                                ui.label(egui::RichText::new("MAC").weak().size(16.0));
                                ui.label(egui::RichText::new(mac).monospace().size(16.0));
                                ui.end_row();
                            }
                            if let Some(ref driver) = kb.device.driver {
                                ui.label(egui::RichText::new("Driver").weak().size(16.0));
                                ui.label(egui::RichText::new(driver.as_str()).size(16.0));
                                ui.end_row();
                            }
                            if let Some(ref host) = kb.bluetooth.paired_host_addr {
                                ui.label(egui::RichText::new("Paired host").weak().size(16.0));
                                ui.label(egui::RichText::new(host).monospace().size(16.0));
                                ui.end_row();
                            }
                        });
                    });

                    // RIGHT: Firmware
                    cols[1].group(|ui| {
                        ui.label(egui::RichText::new("Firmware").strong().size(18.0));
                        ui.add_space(4.0);
                        egui::Grid::new("dev_right").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            if let Some(ref chip) = kb.device.chip {
                                ui.label(egui::RichText::new("Chip").weak().size(16.0));
                                ui.label(egui::RichText::new(chip.as_str()).size(16.0));
                                ui.end_row();
                            }
                            if let Some(ref fw) = kb.firmware.version {
                                ui.label(egui::RichText::new("Version (0x4F)").weak().size(16.0));
                                ui.label(egui::RichText::new(fw).strong().size(18.0));
                                ui.end_row();
                            }
                            // Uninterpreted vendor reports (meaning not proven, #131/#132).
                            for (id, hex) in &kb.raw {
                                ui.label(egui::RichText::new(format!("{id} (raw)")).weak().size(16.0));
                                ui.label(egui::RichText::new(hex).monospace().size(16.0));
                                ui.end_row();
                            }
                            if kb.incomplete {
                                ui.label(egui::RichText::new("Read").weak().size(16.0));
                                ui.label(egui::RichText::new("incomplete (timeout)").size(16.0));
                                ui.end_row();
                            }
                        });
                    });
                });

                ui.add_space(8.0);

                // ── Battery History Graph ─────────────────────────────
                self.draw_battery_history(ui);
            }
        }
        }); // ScrollArea
    }

    /// Editable name of the keyboard: text field + Rename / Reset (BlueZ alias).
    fn rename_row(&mut self, ui: &mut egui::Ui, mac: &str, current: Option<&str>) {
        let current = current.unwrap_or_default().to_string();
        ui.vertical(|ui| {
            let mut submit: Option<String> = None;
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.rename_buf)
                        .desired_width(220.0)
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
                ui.label(if ok { t.weak() } else { t.color(self.palette.bad) });
            }
        });
    }

    /// Draw battery + voltage history chart using egui painter.
    fn draw_battery_history(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Battery History").strong().size(18.0));
                if ui.button(egui::RichText::new("Refresh").size(14.0)).clicked() {
                    let entries = source::load_history();
                    self.battery_history = entries.iter().map(|e| (e.ts as f64, e.pct)).collect();
                    self.voltage_history = entries.iter().filter_map(|e| e.voltage.map(|v| (e.ts as f64, v))).collect();
                }
            });

            if self.battery_history.is_empty() {
                ui.label(egui::RichText::new("No history data yet.").weak().size(16.0));
                return;
            }

            // Info line
            let n = self.battery_history.len();
            let ts_first = self.battery_history.first().map(|p| p.0).unwrap_or(0.0);
            let ts_last = self.battery_history.last().map(|p| p.0).unwrap_or(0.0);
            let span_hours = (ts_last - ts_first) / 3600.0;
            ui.label(egui::RichText::new(
                format!("{} data points, spanning {:.1} hours", n, span_hours)
            ).weak().size(14.0));

            ui.add_space(4.0);

            // Chart area: reserve 160px height
            let chart_height = 160.0;
            let (response, painter) = ui.allocate_painter(
                egui::Vec2::new(ui.available_width(), chart_height),
                egui::Sense::hover(),
            );
            let rect = response.rect;

            // Background
            let vis = ui.visuals().clone();
            painter.rect_filled(rect, 4.0, vis.extreme_bg_color);

            // Margins inside the chart
            let margin = 8.0;
            let plot_rect = egui::Rect::from_min_max(
                egui::Pos2::new(rect.min.x + margin, rect.min.y + margin),
                egui::Pos2::new(rect.max.x - margin, rect.max.y - margin),
            );

            if plot_rect.width() < 10.0 || plot_rect.height() < 10.0 {
                return;
            }

            // Filter to last 24h if data spans more
            let now_ts = unsafe { libc::time(std::ptr::null_mut()) } as f64;
            let cutoff = now_ts - 24.0 * 3600.0;
            let batt_data: Vec<(f64, f64)> = self.battery_history.iter()
                .filter(|(ts, _)| *ts >= cutoff)
                .copied()
                .collect();
            let volt_data: Vec<(f64, f64)> = self.voltage_history.iter()
                .filter(|(ts, _)| *ts >= cutoff)
                .copied()
                .collect();

            if batt_data.len() < 2 {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Not enough data points",
                    egui::FontId::proportional(14.0),
                    vis.weak_text_color(),
                );
                return;
            }

            // Time range
            let t_min = batt_data.first().unwrap().0;
            let t_max = batt_data.last().unwrap().0;
            let t_range = (t_max - t_min).max(1.0);

            // Battery Y range: 0-100
            let batt_y_min = 0.0_f64;
            let batt_y_max = 100.0_f64;

            // Voltage Y range: dynamic from data
            let v_min = volt_data.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let v_max = volt_data.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            let v_range = (v_max - v_min).max(0.1);
            let v_lo = v_min - v_range * 0.05;
            let v_hi = v_max + v_range * 0.05;

            // Map functions
            let map_batt = |ts: f64, pct: f64| -> egui::Pos2 {
                let x = plot_rect.min.x + ((ts - t_min) / t_range) as f32 * plot_rect.width();
                let y = plot_rect.max.y - ((pct - batt_y_min) / (batt_y_max - batt_y_min)) as f32 * plot_rect.height();
                egui::Pos2::new(x, y)
            };
            let map_volt = |ts: f64, v: f64| -> egui::Pos2 {
                let x = plot_rect.min.x + ((ts - t_min) / t_range) as f32 * plot_rect.width();
                let y = plot_rect.max.y - ((v - v_lo) / (v_hi - v_lo)) as f32 * plot_rect.height();
                egui::Pos2::new(x, y)
            };

            // Horizontal grid lines for battery (0%, 25%, 50%, 75%, 100%)
            for &level in &[0.0, 25.0, 50.0, 75.0, 100.0] {
                let y = map_batt(t_min, level).y;
                painter.line_segment(
                    [egui::Pos2::new(plot_rect.min.x, y), egui::Pos2::new(plot_rect.max.x, y)],
                    egui::Stroke::new(0.5, vis.widgets.noninteractive.bg_stroke.color),
                );
                painter.text(
                    egui::Pos2::new(plot_rect.min.x + 2.0, y - 10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}%", level),
                    egui::FontId::proportional(10.0),
                    vis.weak_text_color(),
                );
            }

            // Draw battery % line (green)
            let batt_color = self.palette.good;
            for pair in batt_data.windows(2) {
                let p0 = map_batt(pair[0].0, pair[0].1);
                let p1 = map_batt(pair[1].0, pair[1].1);
                painter.line_segment([p0, p1], egui::Stroke::new(2.0, batt_color));
            }

            // Draw voltage line (cyan)
            let volt_color = self.palette.info;
            for pair in volt_data.windows(2) {
                let p0 = map_volt(pair[0].0, pair[0].1);
                let p1 = map_volt(pair[1].0, pair[1].1);
                painter.line_segment([p0, p1], egui::Stroke::new(1.5, volt_color));
            }

            // Legend
            let legend_y = plot_rect.min.y + 4.0;
            painter.text(
                egui::Pos2::new(plot_rect.max.x - 4.0, legend_y),
                egui::Align2::RIGHT_TOP,
                format!("Battery %  |  Voltage ({:.2}-{:.2} V)", v_min, v_max),
                egui::FontId::proportional(11.0),
                vis.weak_text_color(),
            );
            // Color swatches for legend
            let swatch_y = legend_y + 2.0;
            let swatch_size = 8.0;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::Pos2::new(plot_rect.max.x - 200.0, swatch_y),
                    egui::Vec2::new(swatch_size, swatch_size),
                ),
                0.0, batt_color,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::Pos2::new(plot_rect.max.x - 100.0, swatch_y),
                    egui::Vec2::new(swatch_size, swatch_size),
                ),
                0.0, volt_color,
            );
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
            let keyd_ok = std::path::Path::new("/etc/keyd/apple-keyboard.conf").exists();
            out.push(DiagResult {
                label: "keyd config".into(), ok: keyd_ok,
                detail: if keyd_ok { "/etc/keyd/apple-keyboard.conf".into() } else { "NOT FOUND".into() },
            });

            // udev rules
            let udev_ok = std::path::Path::new("/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules").exists();
            out.push(DiagResult {
                label: "udev rules".into(), ok: udev_ok,
                detail: if udev_ok { "70-apple-kb-hidraw.rules installed".into() } else { "NOT FOUND".into() },
            });

            // modprobe
            let mod_ok = std::path::Path::new("/etc/modprobe.d/hid_apple.conf").exists();
            out.push(DiagResult {
                label: "hid_apple fnmode".into(), ok: mod_ok,
                detail: if mod_ok { "fnmode=1 configured".into() } else { "NOT FOUND — media keys won't be default".into() },
            });

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

        egui::ScrollArea::vertical().show(ui, |ui| {
            for r in &results {
                ui.horizontal(|ui| {
                    let (icon, c) = if r.ok { ("OK", self.palette.good) } else { ("FAIL", self.palette.bad) };
                    ui.label(egui::RichText::new(icon).size(16.0).strong().color(c));
                    ui.label(egui::RichText::new(&r.label).strong().size(16.0));
                    ui.label(egui::RichText::new(&r.detail).weak().size(16.0));
                });
            }
        });
    }
}

// ── Entrypoint ──────────────────────────────────────────────────────────────

/// Open the window and block until it is closed. Returns false when it
/// could not be opened (no display / GPU).
fn open_window(state: &State, raise: &Arc<AtomicBool>, quit_flag: &Arc<AtomicBool>, open: &Arc<AtomicBool>) -> bool {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Apple Keyboard Monitor")
            .with_app_id(instance::APP_ID)
            .with_inner_size([720.0, 600.0])
            .with_min_inner_size([500.0, 400.0]),
        vsync: true,
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
    // On-demand window: `apihub-app` or D-Bus `org.freedesktop.Application`
    // Activate. A second launch raises the running window and exits. The tray
    // belongs to the daemon; only when the daemon does not provide one does
    // this process keep the legacy tray (and stay alive after the window).
    let window_open = Arc::new(AtomicBool::new(false));
    let raise = Arc::new(AtomicBool::new(false));
    let (ui_tx, ui_rx) = mpsc::channel();
    let activate = {
        let (open, raise, tx) = (window_open.clone(), raise.clone(), Mutex::new(ui_tx.clone()));
        move || {
            if open.load(Ordering::Relaxed) {
                raise.store(true, Ordering::Relaxed);
            } else {
                let _ = tx.lock().map(|t| t.send(tray::UiCmd::ShowWindow));
            }
        }
    };
    let _conn = match instance::claim(activate) {
        instance::Claim::Existing => {
            eprintln!("[apihub] already running: window raised");
            return;
        }
        instance::Claim::Primary(c) => Some(c),
        instance::Claim::NoBus => None,
    };

    let state: State = Arc::new(Watch::new());
    let quit_flag = Arc::new(AtomicBool::new(false));
    let src = source::spawn(state.clone());

    let legacy_tray = !instance::daemon_tray_present();
    if legacy_tray {
        eprintln!("[apihub] no daemon tray: legacy tray kept in this process");
        tray::spawn(state.clone(), raise.clone(), quit_flag.clone(), ui_tx);
    }

    let shown = open_window(&state, &raise, &quit_flag, &window_open);
    // Legacy mode only: stay as tray after the window, unless the daemon has
    // taken over the tray in the meantime or the user quit.
    if legacy_tray && !quit_flag.load(Ordering::Relaxed) && !instance::daemon_tray_present() {
        eprintln!("[apihub] window closed — back to tray mode");
        while let Ok(cmd) = ui_rx.recv() {
            match cmd {
                tray::UiCmd::Quit => break,
                tray::UiCmd::ShowWindow => {
                    open_window(&state, &raise, &quit_flag, &window_open);
                    if quit_flag.load(Ordering::Relaxed) || instance::daemon_tray_present() {
                        break;
                    }
                    // Clicks received while the window was open: not a reopen request.
                    if std::iter::from_fn(|| ui_rx.try_recv().ok()).any(|c| c == tray::UiCmd::Quit) {
                        break;
                    }
                    eprintln!("[apihub] window closed — back to tray mode");
                }
            }
        }
    }
    eprintln!("[apihub] shutting down");
    src.stop();
    if !shown {
        std::process::exit(1);
    }
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
