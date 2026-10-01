mod bluez;
mod history;
mod keyboard;
mod power;
mod rssi;
mod tray;

use keyboard::*;

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;

// ── Shared state ────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub(crate) struct SharedState {
    pub(crate) keyboard: Option<KbReport>,
    pub(crate) kb_error: Option<String>,
    pub(crate) caps_lock: bool,
    pub(crate) num_lock: bool,
    pub(crate) remaining_display: Option<String>,
}

type State = Arc<Mutex<SharedState>>;

/// Tray tooltip text; shows "n/a" instead of an invented 0 when the source is absent.
fn tooltip_text(snap: &SharedState) -> String {
    let pct = snap.keyboard.as_ref().and_then(|kb| {
        kb.battery.percentage_fine
            .or(kb.battery.percentage_interpolated)
            .or(kb.battery.percentage)
    });
    format!(
        "Apple Keyboard \u{2014} Battery: {}",
        pct.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "n/a".into()),
    )
}

// ── Background polling ──────────────────────────────────────────────────────

/// Start the polling thread under a supervisor: if the worker panics, the
/// shared-state poison is cleared and the worker is restarted (it used to die
/// silently, freezing the UI and the tray on stale data).
fn spawn_poll_thread(state: State, quit_flag: Arc<AtomicBool>) {
    // Wake event monitor (Input Report 0x13)
    let _wake_monitor = keyboard::find_apple_hidraw()
        .map(|path| keyboard::spawn_wake_monitor(&path));

    thread::Builder::new()
        .name("poll-supervisor".into())
        .spawn(move || {
            while !quit_flag.load(Ordering::Relaxed) {
                let (st, qf) = (state.clone(), quit_flag.clone());
                let worker = thread::Builder::new()
                    .name("poll".into())
                    .spawn(move || poll_loop(st, qf))
                    .expect("failed to spawn poll thread");
                match worker.join() {
                    Ok(()) => break, // quit flag honoured
                    Err(_) => {
                        eprintln!("[poll] worker panicked — restarting in 5s");
                        state.clear_poison();
                        if let Ok(mut s) = state.lock() {
                            s.kb_error = Some("Poll thread crashed — restarting".into());
                        }
                        thread::sleep(Duration::from_secs(5));
                    }
                }
            }
        })
        .expect("failed to spawn poll supervisor");
}

fn poll_loop(state: State, quit_flag: Arc<AtomicBool>) {
    let mut cycle: u32 = 0;
    let mut battery_notified = false;

    // BlueZ Battery Provider — lazy init: created when keyboard is first found.
    // This avoids None-forever if the keyboard is absent at startup (M6).
    let mut battery_provider: Option<bluez::BatteryProvider> = None;
    // MAC currently exported on the BlueZ provider (withdrawn on disconnection).
    let mut provider_mac: Option<String> = None;
    // Kernel power_supply percentage — source of truth when available.
    let mut kernel_pct: Option<u8> = None;

    loop {
        // Check quit flag before each cycle
        if quit_flag.load(Ordering::Relaxed) {
            eprintln!("[poll] quit flag set, exiting poll thread");
            break;
        }
        // ── Keyboard: direct HID ioctl — every 4th cycle (~40s) ──
        let prev_kb = state.lock().ok().and_then(|s| s.keyboard.clone());
        let mut kb = if cycle.is_multiple_of(4) {
            let mut fresh = keyboard::read_keyboard();
            // RSSI is only refreshed every 10th cycle: carry the last known
            // values over a HID re-read so they don't flicker to "absent".
            if let (Some(nk), Some(pk)) = (fresh.as_mut(), prev_kb.as_ref()) {
                if nk.radio.rssi_dbm.is_none() { nk.radio.rssi_dbm = pk.radio.rssi_dbm; }
                if nk.radio.tx_power_dbm.is_none() { nk.radio.tx_power_dbm = pk.radio.tx_power_dbm; }
            }
            fresh
        } else {
            prev_kb
        };
        let mut mac_for_rssi: Option<String> = None;

        // Kernel battery (power_supply) every 3rd cycle (~30s, like UPower: each
        // read triggers a HID GET_REPORT). It overrides the raw HID percentage.
        let kb_mac = kb.as_ref().and_then(|k| k.device.mac.clone());
        if kb.is_none() {
            kernel_pct = None;
        } else if cycle.is_multiple_of(3) {
            kernel_pct = kb_mac.as_deref()
                .and_then(power::kernel_battery)
                .map(|r| r.percent);
        }
        if let (Some(k), Some(p)) = (kb.as_mut(), kernel_pct) {
            k.battery.percentage_fine = Some(f64::from(p));
        }

        // Keyboard gone (or another one): withdraw the exported Battery1 so
        // BlueZ/KDE do not keep showing a frozen value.
        let connected_mac = kb.as_ref()
            .filter(|k| k.bluetooth.connected)
            .and_then(|k| k.device.mac.clone());
        if let (Some(old), Some(bp)) = (provider_mac.as_ref(), battery_provider.as_ref()) {
            if connected_mac.as_ref() != Some(old) {
                bp.remove(old);
                provider_mac = None;
            }
        }

        // Battery low notification + BlueZ provider update + history
        if let Some(ref k) = kb {
            let pct_opt = k.battery.percentage_fine
                .or(k.battery.percentage_interpolated)
                .or(k.battery.percentage)
                .filter(|p| p.is_finite());
            let pct = pct_opt.unwrap_or(100.0);

            // BlueZ Battery Provider: created lazily, then one object per keyboard.
            if let (Some(mac), true) = (connected_mac.as_deref(), pct_opt.is_some()) {
                if battery_provider.is_none() {
                    battery_provider = bluez::BatteryProvider::spawn();
                }
                if let Some(bp) = battery_provider.as_ref() {
                    bp.set_battery(mac, pct.round().clamp(0.0, 100.0) as u8);
                    provider_mac = Some(mac.to_string());
                }
            }

            // Save MAC for RSSI read after this borrow ends
            mac_for_rssi = k.device.mac.clone();

            // History logging (every 30th cycle = ~15s)
            if cycle.is_multiple_of(30) && pct_opt.is_some() {
                // Only log real samples (no invented 100 % / 0 V points).
                if let Some(voltage) = k.battery.voltage {
                    history::append_history(pct, voltage);
                }
            }

            // Re-arm the low-battery alert once the battery has recovered
            if pct_opt.is_some() && pct >= 20.0 {
                battery_notified = false;
            }
            // Low battery notification
            if pct_opt.is_some() && !battery_notified && pct < 15.0 {
                battery_notified = true;
                let _ = notify_rust::Notification::new()
                    .summary("Apple Keyboard — Low Battery")
                    .body(&format!("Battery at {:.0}% — charge soon", pct))
                    .icon("battery-caution")
                    .show();
                // Flash CapsLock LED 5 times as visual alert
                keyboard::flash_capslock(5);
            }
        }

        // RSSI from BlueZ MGMT API (every 10th cycle — blocking socket)
        if cycle.is_multiple_of(10) {
            if let Some(ref mac) = mac_for_rssi {
                if let Some((rssi, tx)) = rssi::read_rssi(mac) {
                    if let Some(ref mut k) = kb {
                        k.radio.rssi_dbm = Some(rssi as i32);
                        k.radio.tx_power_dbm = Some(tx as i32);
                    }
                }
            }
        }

        // LED state (sysfs, fast)
        let (caps, num) = keyboard::read_led_state();

        // Battery remaining (every 30th cycle)
        // Outer Option: "was recomputed this cycle"; inner: the estimate
        // (None clears a stale value, e.g. after a recharge).
        let remaining: Option<Option<String>> = if cycle.is_multiple_of(30) {
            Some(history::estimate_remaining().map(|(rate, hours)| {
                if hours < 24.0 {
                    format!("{:.1}h ({:.1} mV/h)", hours, rate)
                } else {
                    format!("{:.1} days ({:.1} mV/h)", hours / 24.0, rate)
                }
            }))
        } else {
            None
        };

        if let Ok(mut s) = state.lock() {
            s.kb_error = if kb.is_none() { Some("Keyboard: not found".into()) } else { None };
            s.keyboard = kb;
            s.caps_lock = caps;
            s.num_lock = num;
            if let Some(r) = remaining {
                s.remaining_display = r;
            }
        }


        cycle = cycle.wrapping_add(1);
        thread::sleep(Duration::from_secs(10));
    }
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
    // System tray tooltip (shared with tray thread)
    tray_tooltip: Arc<Mutex<String>>,
    // Tray "Show Window" flag
    tray_show_window: Arc<AtomicBool>,
    // Battery history graph
    battery_history: Vec<(f64, f64)>,    // (timestamp, percentage)
    voltage_history: Vec<(f64, f64)>,    // (timestamp, voltage)
}

impl ApiHubApp {
    /// Create the GUI app. Does NOT spawn the poll thread or BlueZ —
    /// those are owned by main() and shared via Arc<Mutex<SharedState>>.
    fn new(
        _cc: &eframe::CreationContext<'_>,
        tray_tooltip: Arc<Mutex<String>>,
        state: State,
        tray_show_window: Arc<AtomicBool>,
        quit_flag: Arc<AtomicBool>,
    ) -> Self {
        // Load battery history from disk (once at startup)
        let entries = history::read_history();
        let battery_history: Vec<(f64, f64)> = entries
            .iter()
            .map(|e| (e.ts as f64, e.pct))
            .collect();
        let voltage_history: Vec<(f64, f64)> = entries
            .iter()
            .map(|e| (e.ts as f64, e.voltage))
            .collect();

        Self {
            state,
            tab: Tab::Keyboard,
            style_initialized: false,
            diag_results: Arc::new(Mutex::new(Vec::new())),
            diag_running: Arc::new(AtomicBool::new(false)),
            quit_flag,
            tray_tooltip,
            tray_show_window,
            battery_history,
            voltage_history,
        }
    }
}

impl eframe::App for ApiHubApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tray "Quit" while the window is open: close it so main() can exit.
        if self.quit_flag.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // ── Update tray tooltip (every frame, lightweight) ─────
        {
            let snap_for_tray = self.state.lock().map(|s| s.clone()).ok();
            if let Some(ref snap) = snap_for_tray {
                let text = tooltip_text(snap);
                if let Ok(mut tt) = self.tray_tooltip.lock() {
                    *tt = text;
                }
            }
        }

        // Dark theme + enforce 16px minimum — once only
        if !self.style_initialized {
            ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*ctx.style()).clone();
            for (_text_style, font_id) in style.text_styles.iter_mut() {
                if font_id.size < 16.0 {
                    font_id.size = 16.0;
                }
            }
            style.visuals.window_rounding = egui::Rounding::same(8.0);
            style.visuals.widgets.noninteractive.rounding = egui::Rounding::same(4.0);
            style.visuals.widgets.inactive.rounding = egui::Rounding::same(4.0);
            style.visuals.widgets.hovered.rounding = egui::Rounding::same(4.0);
            style.visuals.widgets.active.rounding = egui::Rounding::same(4.0);
            style.spacing.item_spacing = egui::Vec2::new(8.0, 6.0);
            ctx.set_style(style);
            self.style_initialized = true;
        }

        // Check if tray requested window show
        if self.tray_show_window.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // Next repaint in 2s — egui sleeps until then or until user interaction
        ctx.request_repaint_after(Duration::from_secs(2));

        let snap = self.state.lock().map(|s| s.clone()).unwrap_or_default();

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

// ── Tabs ────────────────────────────────────────────────────────────────────

impl ApiHubApp {
    fn tab_keyboard(&mut self, ui: &mut egui::Ui, snap: &SharedState) {
        if let Some(ref err) = snap.kb_error {
            ui.label(egui::RichText::new(err.as_str()).size(16.0).color(egui::Color32::from_rgb(255, 100, 100)));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
        match &snap.keyboard {
            None => {
                ui.label(egui::RichText::new("Waiting for keyboard data...").size(16.0));
            }
            Some(kb) => {
                let pct = kb.battery.percentage_fine
                    .or(kb.battery.percentage_interpolated)
                    .or(kb.battery.percentage)
                    .unwrap_or(0.0);

                // Top row: battery tile + radio tile side by side
                ui.columns(2, |cols| {
                    // LEFT: Battery tile
                    cols[0].group(|ui| {
                        ui.vertical_centered(|ui| {
                            let color = if pct > 50.0 {
                                egui::Color32::from_rgb(80, 220, 100)
                            } else if pct > 20.0 {
                                egui::Color32::from_rgb(255, 200, 50)
                            } else {
                                egui::Color32::from_rgb(255, 70, 70)
                            };
                            ui.colored_label(color,
                                egui::RichText::new(format!("{:.0}%", pct)).size(28.0).strong());
                            // Battery type subtitle under hero percentage
                            if let Some(v) = kb.battery.voltage {
                                ui.label(egui::RichText::new(keyboard::detect_battery_type(v))
                                    .weak().size(16.0));
                            }
                            ui.add(egui::ProgressBar::new((pct / 100.0).clamp(0.0, 1.0) as f32)
                                .text(format!("{:.1}%", pct)));
                        });
                        ui.add_space(4.0);
                        egui::Grid::new("bat_detail").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            if let Some(v) = kb.battery.voltage {
                                ui.label(egui::RichText::new("Voltage").weak().size(16.0));
                                // Colored voltage indicator
                                let v_color = if v > 2.8 {
                                    egui::Color32::from_rgb(80, 220, 100)
                                } else if v > 2.4 {
                                    egui::Color32::from_rgb(255, 200, 50)
                                } else {
                                    egui::Color32::from_rgb(255, 70, 70)
                                };
                                ui.label(egui::RichText::new(format!("{:.3} V", v)).strong().size(18.0).color(v_color));
                                ui.end_row();
                            }
                            if let Some(adc) = kb.battery.adc_raw {
                                ui.label(egui::RichText::new("ADC").weak().size(16.0));
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
                                let caps_color = if snap.caps_lock { egui::Color32::from_rgb(80, 220, 100) } else { egui::Color32::GRAY };
                                let num_color = if snap.num_lock { egui::Color32::from_rgb(80, 220, 100) } else { egui::Color32::GRAY };
                                ui.label(egui::RichText::new("CAPS").size(16.0).color(caps_color).strong());
                                ui.label(egui::RichText::new("NUM").size(16.0).color(num_color).strong());
                            });
                            ui.end_row();
                        });
                    });

                    // RIGHT: Radio tile
                    cols[1].group(|ui| {
                        ui.label(egui::RichText::new("Radio").strong().size(18.0));
                        ui.add_space(4.0);
                        egui::Grid::new("radio_detail").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            let rssi = kb.radio.rssi_dbm.or(kb.bluetooth.rssi_dbus);
                            if let Some(r) = rssi {
                                ui.label(egui::RichText::new("RSSI").weak().size(16.0));
                                let color = if r > -60 {
                                    egui::Color32::from_rgb(80, 220, 100)
                                } else if r > -80 {
                                    egui::Color32::from_rgb(255, 200, 50)
                                } else {
                                    egui::Color32::from_rgb(255, 70, 70)
                                };
                                // Signal bars based on RSSI strength
                                let bars = if r > -50 {
                                    "\u{2582}\u{2584}\u{2586}\u{2588}"
                                } else if r > -60 {
                                    "\u{2582}\u{2584}\u{2586}\u{2581}"
                                } else if r > -70 {
                                    "\u{2582}\u{2584}\u{2581}\u{2581}"
                                } else if r > -80 {
                                    "\u{2582}\u{2581}\u{2581}\u{2581}"
                                } else {
                                    "\u{2581}\u{2581}\u{2581}\u{2581}"
                                };
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, egui::RichText::new(format!("{} dBm", r)).strong().size(18.0));
                                    ui.colored_label(color, egui::RichText::new(bars).size(18.0));
                                });
                                ui.end_row();
                            }
                            if let Some(tx) = kb.radio.tx_power_dbm.or(kb.bluetooth.tx_power_dbus) {
                                ui.label(egui::RichText::new("TX Power").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{} dBm", tx)).size(16.0));
                                ui.end_row();
                            }
                            ui.label(egui::RichText::new("Connected").weak().size(16.0));
                            let (txt, col) = if kb.bluetooth.connected {
                                ("Yes", egui::Color32::from_rgb(80, 220, 100))
                            } else {
                                ("No", egui::Color32::from_rgb(255, 70, 70))
                            };
                            ui.label(egui::RichText::new(txt).strong().size(16.0).color(col));
                            ui.end_row();

                            ui.label(egui::RichText::new("Paired").weak().size(16.0));
                            ui.label(egui::RichText::new(if kb.bluetooth.paired { "Yes" } else { "No" }).size(16.0));
                            ui.end_row();

                            // Compact BT Interval/Timeout on one row
                            if let Some(interval) = kb.bluetooth.conn_interval_ms {
                                let latency = kb.bluetooth.slave_latency.unwrap_or(0);
                                let effective = interval * (latency as f64 + 1.0);
                                let timeout_str = kb.bluetooth.supervision_timeout_s
                                    .map(|t| format!(" | T/O {:.1}s", t))
                                    .unwrap_or_default();
                                ui.label(egui::RichText::new("BT Link").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{:.0}ms eff={:.0}ms{}", interval, effective, timeout_str)).size(16.0));
                                ui.end_row();
                            } else if let Some(timeout) = kb.bluetooth.supervision_timeout_s {
                                ui.label(egui::RichText::new("BT Timeout").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{:.1}s", timeout)).size(16.0));
                                ui.end_row();
                            }
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
                                ui.label(egui::RichText::new("Name").weak().size(16.0));
                                ui.label(egui::RichText::new(name).size(16.0));
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
                            if let Some(ref key) = kb.bluetooth.identity_key {
                                ui.label(egui::RichText::new("Identity").weak().size(16.0));
                                let short: String = key.chars().take(23).collect();
                                ui.label(egui::RichText::new(short).monospace().size(16.0));
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
                                ui.label(egui::RichText::new("Version").weak().size(16.0));
                                ui.label(egui::RichText::new(fw).strong().size(18.0));
                                ui.end_row();
                            }
                            if let Some(ref build) = kb.firmware.build {
                                ui.label(egui::RichText::new("Build").weak().size(16.0));
                                ui.label(egui::RichText::new(build.to_string()).size(16.0));
                                ui.end_row();
                            }
                            if let Some(adc_ref) = kb.firmware.adc_ref {
                                ui.label(egui::RichText::new("ADC Ref").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{}", adc_ref)).monospace().size(16.0));
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

    /// Draw battery + voltage history chart using egui painter.
    fn draw_battery_history(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Battery History").strong().size(18.0));
                if ui.button(egui::RichText::new("Refresh").size(14.0)).clicked() {
                    let entries = history::read_history();
                    self.battery_history = entries.iter().map(|e| (e.ts as f64, e.pct)).collect();
                    self.voltage_history = entries.iter().map(|e| (e.ts as f64, e.voltage)).collect();
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
            painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 22, 28));

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
                    egui::Color32::GRAY,
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
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(50, 52, 58)),
                );
                painter.text(
                    egui::Pos2::new(plot_rect.min.x + 2.0, y - 10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}%", level),
                    egui::FontId::proportional(10.0),
                    egui::Color32::from_rgb(100, 100, 110),
                );
            }

            // Draw battery % line (green)
            let batt_color = egui::Color32::from_rgb(80, 220, 100);
            for pair in batt_data.windows(2) {
                let p0 = map_batt(pair[0].0, pair[0].1);
                let p1 = map_batt(pair[1].0, pair[1].1);
                painter.line_segment([p0, p1], egui::Stroke::new(2.0, batt_color));
            }

            // Draw voltage line (cyan)
            let volt_color = egui::Color32::from_rgb(100, 180, 255);
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
                egui::Color32::from_rgb(140, 140, 150),
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
                ("apple-kb-monitor", vec!["--version".into()], "Main daemon binary"),
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

            // Service
            let active = Command::new("systemctl")
                .args(["--user", "is-active", "apple-kb-monitor.service"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            out.push(DiagResult {
                label: "apple-kb-monitor.service".into(), ok: active,
                detail: if active { "active (running)".into() } else { "inactive / not found".into() },
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
                ui.label(egui::RichText::new("Running...").size(16.0)
                    .color(egui::Color32::from_rgb(255, 200, 50)));
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
                .color(if fail_count == 0 { egui::Color32::from_rgb(80, 220, 100) }
                       else { egui::Color32::from_rgb(255, 200, 50) }));
            if fail_count > 0 {
                ui.label(egui::RichText::new(format!("  {} issues", fail_count)).size(16.0)
                    .color(egui::Color32::from_rgb(255, 70, 70)));
            }
        });

        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for r in &results {
                ui.horizontal(|ui| {
                    let icon = if r.ok { "\u{2705}" } else { "\u{274C}" };
                    ui.label(egui::RichText::new(icon).size(16.0));
                    ui.label(egui::RichText::new(&r.label).strong().size(16.0));
                    ui.label(egui::RichText::new(&r.detail).weak().size(16.0));
                });
            }
        });
    }
}

// ── Entrypoint ──────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    // ── Single process architecture ──────────────────────────────────
    // 1. Start tray (always, zero CPU via zbus epoll)
    // 2. Start the supervised poll thread
    // 3. Wait for "Show Window" → open eframe in THIS process
    // 4. When window closes → back to tray-only (no subprocess, no zombie)
    // 5. Quit flag — graceful shutdown, lets destructors run

    let state: State = Arc::new(Mutex::new(SharedState::default()));

    // Shared quit flag — set by tray "Quit", checked by main loop + poll thread
    let quit_flag = Arc::new(AtomicBool::new(false));

    // ── Tray icon ────────────────────────────────────────────────────
    let tray_tooltip: Arc<Mutex<String>> = Arc::new(Mutex::new("Apple Keyboard \u{2014} starting...".into()));
    let show_window = Arc::new(AtomicBool::new(false));
    tray::spawn(
        tray_tooltip.clone(),
        state.clone(),
        show_window.clone(),
        quit_flag.clone(),
    );

    // ── Polling ──────────────────────────────────────────────────────
    spawn_poll_thread(Arc::clone(&state), quit_flag.clone());

    eprintln!("[apihub] tray mode — click scarab icon to open window");

    // ── Main loop: tray-only until "Show Window" ─────────────────────
    loop {
        // Check quit flag — break out and let destructors run
        if quit_flag.load(Ordering::Relaxed) {
            eprintln!("[apihub] quit flag set — shutting down gracefully");
            break;
        }

        std::thread::sleep(Duration::from_secs(2));

        // Update tray tooltip
        if let Ok(snap) = state.lock() {
            if let Ok(mut tt) = tray_tooltip.lock() {
                *tt = tooltip_text(&snap);
            }
        }

        // Show Window → open eframe in THIS process (blocks until window closed)
        if show_window.swap(false, Ordering::Relaxed) {
            eprintln!("[apihub] opening window...");
            let options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title("Apple Keyboard Monitor")
                    .with_inner_size([720.0, 600.0])
                    .with_min_inner_size([500.0, 400.0]),
                vsync: true,
                ..Default::default()
            };
            // eframe::run_native blocks until the window is closed
            if let Err(e) = eframe::run_native(
                "apihub",
                options,
                Box::new({
                    let tt = tray_tooltip.clone();
                    let st = state.clone();
                    let sw = show_window.clone();
                    let qf = quit_flag.clone();
                    move |cc| Ok(Box::new(ApiHubApp::new(cc, tt, st, sw, qf)))
                }),
            ) {
                // No display / GPU init failure: stay in tray mode instead of dying silently.
                eprintln!("[apihub] cannot open window: {}", e);
            }
            eprintln!("[apihub] window closed — back to tray mode");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_shows_na_without_sources() {
        let t = tooltip_text(&SharedState::default());
        assert!(t.contains("n/a"));
        assert!(!t.contains("0%"));
    }

    #[test]
    fn poisoned_state_is_recovered() {
        let st: State = Arc::new(Mutex::new(SharedState::default()));
        let s2 = st.clone();
        let _ = thread::spawn(move || {
            let _g = s2.lock().unwrap();
            panic!("boom");
        })
        .join();
        assert!(st.lock().is_err());
        st.clear_poison();
        assert!(st.lock().is_ok());
    }
}
