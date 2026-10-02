mod diag;
mod diag_tab;
mod fnmode_diag;
mod framestats;
mod heartbeat;
mod history_chart;
mod i18n;
mod history_view;
mod instance;
mod keyboard;
mod keys_tab;
mod portal;
mod rename;
mod source;
mod view;
mod widgets;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use crate::i18n::{tr, trf};
use view::{Level, Palette};
use widgets::{key, kv_grid, signal_bars, tile, value};

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
    Keys,
    Diag,
}

struct ApiHubApp {
    state: State,
    tab: Tab,
    style_initialized: bool,
    diag: diag_tab::DiagTab,
    // Set by a second launch / D-Bus Activate: bring the window to front
    raise: Arc<AtomicBool>,
    appearance: portal::Shared,
    applied: Option<portal::Appearance>,
    palette: Palette,
    // Battery history graph: loaded by a worker thread, only read here.
    history: history_view::Loader,
    // Rename field (#141)
    rename_buf: String,
    rename_loaded: Option<String>,
    rename_status: rename::Status,
    frame_stats: framestats::FrameStats,
    // UI heartbeat (#240): ticked from update(), written by a small thread.
    heartbeat: Option<heartbeat::Heartbeat>,
}

impl ApiHubApp {
    /// Create the GUI app. Does NOT spawn the acquisition — it is owned by
    /// main() and shared through the `Watch`.
    fn new(
        cc: &eframe::CreationContext<'_>,
        state: State,
        raise: Arc<AtomicBool>,
    ) -> Self {
        // Battery history: loaded off the UI thread (D-Bus + disk, #230).
        let history = history_view::Loader::new();
        let ctx = cc.egui_ctx.clone();
        history.request(move || ctx.request_repaint());

        Self {
            state,
            tab: Tab::Keyboard,
            style_initialized: false,
            diag: diag_tab::DiagTab::new(),
            raise,
            appearance: portal::spawn(cc.egui_ctx.clone()),
            applied: None,
            palette: Palette::new(cc.egui_ctx.style().visuals.dark_mode),
            history,
            rename_buf: String::new(),
            rename_loaded: None,
            rename_status: Arc::new(Mutex::new(None)),
            frame_stats: framestats::FrameStats::from_env(),
            heartbeat: heartbeat::Heartbeat::from_env(),
        }
    }
}

impl eframe::App for ApiHubApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let t0 = std::time::Instant::now();
        self.update_ui(ctx);
        let took = t0.elapsed();
        self.frame_stats.record(took, frame.info().cpu_usage);
        if let Some(hb) = self.heartbeat.as_mut() {
            hb.tick(took);
        }
    }
}

impl ApiHubApp {
    fn update_ui(&mut self, ctx: &egui::Context) {
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

        // A second launch or Activate asked for the window
        if self.raise.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // Next repaint in 1 s (also minimised/hidden): drives the UI heartbeat
        // (#240); egui sleeps until then or until user interaction.
        ctx.request_repaint_after(heartbeat::PERIOD);

        let snap = self.state.get();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Keyboard, tr("Keyboard"));
                ui.selectable_value(&mut self.tab, Tab::Keys, tr("Keys"));
                ui.selectable_value(&mut self.tab, Tab::Diag, tr("Diag"));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab {
                Tab::Keyboard => self.tab_keyboard(ui, &snap),
                Tab::Keys => keys_tab::show(ui),
                Tab::Diag => self.diag.show(ui, &self.palette),
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
    visuals.window_corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(4);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);
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
                ui.label(egui::RichText::new(tr("Waiting for keyboard data...")).size(16.0));
                return;
            };
            let now = unix_now();
            // Two tiles per row only when each gets a usable width (#195).
            let wide = view::two_columns(ui.available_width());
            self.tile_row(ui, wide, |me, ui| me.battery_tile(ui, snap, kb, now), |me, ui| me.radio_tile(ui, snap, kb, now));
            ui.add_space(8.0);
            self.tile_row(ui, wide, |me, ui| me.device_tile(ui, snap, kb), |me, ui| me.firmware_tile(ui, kb));
            ui.add_space(8.0);
            history_chart::show(ui, &self.history, &self.palette);
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
                key(ui, tr("Voltage"));
                value(ui, self.tint(egui::RichText::new(view::volts_text(v)).strong().size(18.0), view::voltage_level(v)));
                ui.end_row();
            }
            if let Some(t) = view::estimate_text(&kb.battery) {
                // Charge estimated by the declared chemistry [hypothèse] (#178).
                key(ui, tr("Estimate"));
                value(ui, egui::RichText::new(t).size(16.0));
                ui.end_row();
            }
            if let Some(t) = view::apple_display_text(&kb.battery) {
                // What macOS would show for the same raw value (#213).
                key(ui, tr("Apple display"));
                value(ui, egui::RichText::new(t).size(16.0));
                ui.end_row();
            }
            if let Some(t) = view::thresholds_text(&kb.battery) {
                // Thresholds the keyboard reports (0x60, read once per connection).
                key(ui, tr("Thresholds"));
                value(ui, egui::RichText::new(t).size(16.0));
                ui.end_row();
            }
            if let Some(t) = view::chemistry_text(&kb.battery) {
                key(ui, tr("Batteries"));
                value(ui, egui::RichText::new(t).size(16.0));
                ui.end_row();
            }
            // The kernel % steps down only at reconnections (#179).
            key(ui, tr("Updated"));
            value(ui, egui::RichText::new(view::age_text(snap.update_age_s(now))).size(16.0));
            ui.end_row();
            if let Some(rem) = view::remaining_text(snap, now) {
                key(ui, tr("Remaining"));
                value(ui, egui::RichText::new(rem).size(16.0));
                ui.end_row();
            }
            key(ui, tr("LEDs"));
            ui.horizontal(|ui| {
                let on = |b: bool| if b { Level::Good } else { Level::Unknown };
                let weak = |b: bool, t: egui::RichText| if b { t } else { t.weak() };
                ui.label(weak(snap.caps_lock, self.tint(egui::RichText::new(tr("CAPS")).size(16.0).strong(), on(snap.caps_lock))));
                ui.label(weak(snap.num_lock, self.tint(egui::RichText::new(tr("NUM")).size(16.0).strong(), on(snap.num_lock))));
            });
            ui.end_row();
        });
    }

    fn radio_tile(&mut self, ui: &mut egui::Ui, snap: &Snapshot, kb: &akm_core::report::KbReport, now: u64) {
        ui.label(egui::RichText::new(tr("Radio")).strong().size(18.0));
        ui.add_space(4.0);
        kv_grid(ui, "radio_detail", |ui| {
            // Relative BR/EDR value (dB to the ideal range), not dBm (#174).
            let rssi = kb.radio.rel_db();
            let lvl = view::rssi_level(rssi);
            key(ui, tr("Signal"));
            ui.horizontal(|ui| {
                ui.label(self.tint(egui::RichText::new(view::rssi_text(rssi)).strong().size(18.0), lvl));
                let c = self.palette.color(lvl).unwrap_or_else(|| ui.visuals().text_color());
                signal_bars(ui, view::rssi_bar_count(rssi), c);
            });
            ui.end_row();
            if view::rssi_valid(rssi).is_some() {
                if let Some(age) = snap.rssi_age_s(now) {
                    key(ui, tr("Measured"));
                    value(ui, egui::RichText::new(view::age_text(Some(age))).size(16.0));
                    ui.end_row();
                }
            }
            key(ui, tr("TX Power"));
            value(ui, egui::RichText::new(view::tx_power_text(kb.radio.tx_power_dbm)).size(16.0));
            ui.end_row();
            key(ui, tr("Connected"));
            let (txt, lvl) = if kb.bluetooth.connected { (tr("Yes"), Level::Good) } else { (tr("No"), Level::Bad) };
            value(ui, self.tint(egui::RichText::new(txt).strong().size(16.0), lvl));
            ui.end_row();
            if let Some(p) = view::paired_text(&kb.bluetooth) {
                key(ui, tr("Paired"));
                value(ui, egui::RichText::new(p).size(16.0));
                ui.end_row();
            }
            if let Some(w) = view::wake_text(&kb.wake) {
                // Passive listening of input report 0x13.
                key(ui, tr("Last wake"));
                value(ui, egui::RichText::new(w).size(16.0));
                ui.end_row();
            }
        });
    }

    fn device_tile(&mut self, ui: &mut egui::Ui, snap: &Snapshot, kb: &akm_core::report::KbReport) {
        ui.label(egui::RichText::new(tr("Device")).strong().size(18.0));
        ui.add_space(4.0);
        kv_grid(ui, "dev_left", |ui| {
            if let Some(ref model) = kb.device.model {
                key(ui, tr("Model"));
                value(ui, egui::RichText::new(model).strong().size(16.0));
                ui.end_row();
            }
            if let Some(ref name) = kb.device.name {
                key(ui, tr("Own name"));
                value(ui, egui::RichText::new(name).size(16.0));
                ui.end_row();
            }
            if let Some(ref mac) = kb.device.mac {
                key(ui, tr("MAC"));
                value(ui, egui::RichText::new(mac).monospace().size(16.0));
                ui.end_row();
            }
            if let Some(ref driver) = kb.device.driver {
                key(ui, tr("Driver"));
                value(ui, egui::RichText::new(driver.as_str()).size(16.0));
                ui.end_row();
            }
            if let Some(ref host) = kb.bluetooth.paired_host_addr {
                key(ui, tr("Paired host"));
                value(ui, egui::RichText::new(host).monospace().size(16.0));
                ui.end_row();
            }
        });
        // The editor gets the full tile width, below the table (#195).
        if let Some(ref mac) = kb.device.mac {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tr("Name")).weak().size(16.0));
            self.rename_row(ui, mac, snap.display_name());
        }
    }

    fn firmware_tile(&mut self, ui: &mut egui::Ui, kb: &akm_core::report::KbReport) {
        ui.label(egui::RichText::new(tr("Firmware")).strong().size(18.0));
        ui.add_space(4.0);
        // Firmware check against the embedded table (#227); never a flash offer.
        match view::firmware_line(&kb.firmware, crate::i18n::is_french()) {
            Some((line, level)) => {
                ui.add(egui::Label::new(self.tint(egui::RichText::new(line).strong().size(16.0), level)).wrap());
                if let Some(src) = kb.firmware.source.as_deref() {
                    ui.add(egui::Label::new(egui::RichText::new(trf("Source: {} \u{b7} table of {}", &[&src, &kb.firmware.table_date.as_deref().unwrap_or("?")])).weak().size(13.0)).wrap());
                }
            }
            None => {
                ui.label(egui::RichText::new(tr("Firmware: not read yet (read once per connection)")).weak().size(16.0));
            }
        }
        ui.add_space(4.0);
        kv_grid(ui, "dev_right", |ui| {
            if let Some(ref chip) = kb.device.chip {
                key(ui, tr("Chip"));
                value(ui, egui::RichText::new(chip.as_str()).size(16.0));
                ui.end_row();
            }
            // Uninterpreted vendor reports (meaning not proven, #131/#132).
            for (id, hex) in &kb.raw {
                key(ui, &trf("{} (raw)", &[id]));
                value(ui, egui::RichText::new(hex).monospace().size(16.0));
                ui.end_row();
            }
            if kb.incomplete {
                key(ui, tr("Read"));
                value(ui, egui::RichText::new(tr("incomplete (timeout)")).size(16.0));
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
                        .hint_text(tr("Keyboard name")),
                );
                // Follow the daemon's name unless the user is typing.
                if self.rename_loaded.as_deref() != Some(current.as_str()) && !edit.has_focus() {
                    self.rename_buf = current.clone();
                    self.rename_loaded = Some(current.clone());
                }
                let changed = self.rename_buf.trim() != current;
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.add_enabled(changed, egui::Button::new(tr("Rename"))).clicked() || enter) && changed {
                    submit = Some(self.rename_buf.clone());
                }
                if ui.button(tr("Reset")).on_hover_text(tr("Restore the keyboard's own name")).clicked() {
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
}

// ── Entrypoint ──────────────────────────────────────────────────────────────

/// Open the window and block until it is closed. Returns false when it
/// could not be opened (no display / GPU). Called **once** per process:
/// winit does not support a second event loop run reliably (#226).
fn open_window(state: &State, raise: &Arc<AtomicBool>, open: &Arc<AtomicBool>) -> bool {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(crate::i18n::tr("Apple Keyboard Monitor"))
            .with_app_id(instance::APP_ID)
            .with_inner_size([720.0, 600.0])
            .with_min_inner_size([500.0, 400.0]),
        // Never block in eglSwapBuffers: with vsync on, Mesa waits for a
        // Wayland frame callback that a minimized or hidden window never
        // gets, the main thread stops answering pings ("Not responding") and
        // ignores Activate/Quit (#231). Repaints are timer-driven (1 s) or
        // input-driven anyway, so there is nothing to tear.
        vsync: false,
        // Return to main() on close, which then ends the process (#226).
        run_and_return: true,
        ..Default::default()
    };
    let (st, sw) = (state.clone(), raise.clone());
    raise.store(false, Ordering::Relaxed);
    open.store(true, Ordering::Relaxed);
    let r = eframe::run_native(instance::APP_ID, options, Box::new(move |cc| Ok(Box::new(ApiHubApp::new(cc, st, sw)))));
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
    // the D-Bus name. Nothing ever reopens a window by itself. The tray icon
    // belongs to the daemon alone (#62): this process never registers one.
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
    let src = source::spawn(state.clone());

    let shown = open_window(&state, &raise, &window_open);
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
}
