#![allow(dead_code)]
mod ddc;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

// ── Keyboard telemetry (from apple-kb-monitor --json) ───────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
struct KbDevice {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mac: Option<String>,
    #[serde(default)]
    chip: Option<String>,
    #[serde(default)]
    driver: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KbBattery {
    #[serde(default)]
    percentage: Option<f64>,
    #[serde(default)]
    percentage_fine: Option<f64>,
    #[serde(default)]
    percentage_interpolated: Option<f64>,
    #[serde(default)]
    voltage: Option<f64>,
    #[serde(default)]
    adc_raw: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KbBluetooth {
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    paired: bool,
    #[serde(default)]
    rssi_dbus: Option<i32>,
    #[serde(default)]
    tx_power_dbus: Option<i32>,
    #[serde(default)]
    address_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KbRadio {
    #[serde(default)]
    rssi_dbm: Option<i32>,
    #[serde(default)]
    tx_power_dbm: Option<i32>,
    #[serde(default)]
    max_tx_power_dbm: Option<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KbFirmware {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    build: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KbReport {
    #[serde(default)]
    device: KbDevice,
    #[serde(default)]
    battery: KbBattery,
    #[serde(default)]
    bluetooth: KbBluetooth,
    #[serde(default)]
    radio: KbRadio,
    #[serde(default)]
    firmware: KbFirmware,
}

fn read_keyboard_json() -> Option<KbReport> {
    let output = Command::new("apple-kb-monitor")
        .arg("--json")
        .output()
        .ok()?;
    if !output.status.success() {
        eprintln!("[kb] command failed: exit={:?}", output.status.code());
        return None;
    }
    match serde_json::from_slice::<KbReport>(&output.stdout) {
        Ok(report) => Some(report),
        Err(e) => {
            eprintln!("[kb] JSON parse error: {}", e);
            // Fallback: parse as generic Value
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                eprintln!("[kb] JSON is valid, struct mismatch. Top keys: {:?}",
                    v.as_object().map(|o| o.keys().collect::<Vec<_>>()));
            }
            None
        }
    }
}

// ── Shared state ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct DdcValues {
    data: HashMap<String, (u16, u16)>,
    last_update: Option<Instant>,
    error: Option<String>,
}

impl Default for DdcValues {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            last_update: None,
            error: None,
        }
    }
}

#[derive(Clone, Default)]
struct SharedState {
    ddc: DdcValues,
    keyboard: Option<KbReport>,
    kb_error: Option<String>,
}

type State = Arc<Mutex<SharedState>>;

// ── Background polling ──────────────────────────────────────────────────────

fn spawn_poll_thread(state: State) {
    thread::spawn(move || {
        loop {
            // Read DDC
            let ddc_data = ddc::read_all_essential(ddc::DEFAULT_BUS);
            let ddc_err = if ddc_data.is_empty() {
                Some("No DDC data — check /dev/i2c-6 permissions".into())
            } else {
                None
            };

            // Read keyboard
            let kb = read_keyboard_json();
            let kb_err = if kb.is_none() {
                Some("apple-kb-monitor --json failed or not found".into())
            } else {
                None
            };

            if let Ok(mut s) = state.lock() {
                s.ddc.data = ddc_data;
                s.ddc.last_update = Some(Instant::now());
                s.ddc.error = ddc_err;
                s.keyboard = kb;
                s.kb_error = kb_err;
            }

            thread::sleep(Duration::from_secs(5));
        }
    });
}

// ── App ─────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Tab {
    Keyboard,
    Display,
    Advanced,
    System,
}

struct ApiHubApp {
    state: State,
    tab: Tab,
    i2c_bus: String,
    pending_writes: Vec<(u8, u16)>,
}

impl ApiHubApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let state: State = Arc::new(Mutex::new(SharedState::default()));
        spawn_poll_thread(Arc::clone(&state));
        Self {
            state,
            tab: Tab::Keyboard,
            i2c_bus: ddc::DEFAULT_BUS.to_string(),
            pending_writes: Vec::new(),
        }
    }
}

impl eframe::App for ApiHubApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process any pending DDC writes
        for (vcp, val) in self.pending_writes.drain(..) {
            let bus = self.i2c_bus.clone();
            thread::spawn(move || {
                if let Err(e) = ddc::ddc_write_vcp(&bus, vcp, val) {
                    eprintln!("DDC write 0x{:02X}={}: {}", vcp, val, e);
                }
            });
        }

        // Dark theme
        ctx.set_visuals(egui::Visuals::dark());

        // Request repaint every second for live data
        ctx.request_repaint_after(Duration::from_secs(1));

        let snap = self.state.lock().map(|s| s.clone()).unwrap_or_default();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Keyboard, "Keyboard");
                ui.selectable_value(&mut self.tab, Tab::Display, "Display");
                ui.selectable_value(&mut self.tab, Tab::Advanced, "Advanced");
                ui.selectable_value(&mut self.tab, Tab::System, "System");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab {
                Tab::Keyboard => self.tab_keyboard(ui, &snap),
                Tab::Display => self.tab_display(ui, &snap),
                Tab::Advanced => self.tab_advanced(ui, &snap),
                Tab::System => self.tab_system(ui, &snap),
            }
        });
    }
}

// ── Helper macros ───────────────────────────────────────────────────────────

fn ddc_cur(snap: &SharedState, name: &str) -> u16 {
    snap.ddc.data.get(name).map(|v| v.0).unwrap_or(0)
}

fn ddc_max(snap: &SharedState, name: &str) -> u16 {
    snap.ddc.data.get(name).map(|v| v.1).unwrap_or(100)
}

fn ddc_has(snap: &SharedState, name: &str) -> bool {
    snap.ddc.data.contains_key(name)
}

// ── Tabs ────────────────────────────────────────────────────────────────────

impl ApiHubApp {
    fn tab_keyboard(&self, ui: &mut egui::Ui, snap: &SharedState) {
        ui.heading("Apple Keyboard");
        ui.separator();

        if let Some(ref err) = snap.kb_error {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
        }

        match &snap.keyboard {
            None => {
                ui.label("Waiting for keyboard data...");
            }
            Some(kb) => {
                // Battery big display
                let pct = kb.battery.percentage_interpolated
                    .or(kb.battery.percentage_fine)
                    .or(kb.battery.percentage)
                    .unwrap_or(0.0);

                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    let color = if pct > 50.0 {
                        egui::Color32::from_rgb(80, 220, 100)
                    } else if pct > 20.0 {
                        egui::Color32::from_rgb(255, 200, 50)
                    } else {
                        egui::Color32::from_rgb(255, 70, 70)
                    };
                    ui.colored_label(
                        color,
                        egui::RichText::new(format!("{:.0}%", pct)).size(64.0).strong(),
                    );
                    ui.label("Battery");
                    ui.add_space(5.0);

                    let bar_pct = (pct / 100.0).clamp(0.0, 1.0) as f32;
                    ui.add(
                        egui::ProgressBar::new(bar_pct)
                            .desired_width(300.0)
                            .text(format!("{:.1}%", pct)),
                    );
                });

                ui.add_space(15.0);

                egui::Grid::new("kb_info")
                    .num_columns(2)
                    .spacing([20.0, 6.0])
                    .show(ui, |ui| {
                        if let Some(v) = kb.battery.voltage {
                            ui.label("Voltage:");
                            ui.label(format!("{:.3} V", v));
                            ui.end_row();
                        }

                        if let Some(adc) = kb.battery.adc_raw {
                            ui.label("ADC Raw:");
                            ui.label(format!("{}", adc));
                            ui.end_row();
                        }

                        let rssi = kb.radio.rssi_dbm.or(kb.bluetooth.rssi_dbus);
                        if let Some(r) = rssi {
                            ui.label("RSSI:");
                            let color = if r > -60 {
                                egui::Color32::from_rgb(80, 220, 100)
                            } else if r > -80 {
                                egui::Color32::from_rgb(255, 200, 50)
                            } else {
                                egui::Color32::from_rgb(255, 70, 70)
                            };
                            ui.colored_label(color, format!("{} dBm", r));
                            ui.end_row();
                        }

                        if let Some(tx) = kb.radio.tx_power_dbm.or(kb.bluetooth.tx_power_dbus) {
                            ui.label("TX Power:");
                            ui.label(format!("{} dBm", tx));
                            ui.end_row();
                        }

                        if let Some(lq) = kb.radio.max_tx_power_dbm {
                            ui.label("Max TX Power:");
                            ui.label(format!("{}", lq));
                            ui.end_row();
                        }

                        ui.label("Connected:");
                        let (txt, col) = if kb.bluetooth.connected {
                            ("Yes", egui::Color32::from_rgb(80, 220, 100))
                        } else {
                            ("No", egui::Color32::from_rgb(255, 70, 70))
                        };
                        ui.colored_label(col, txt);
                        ui.end_row();

                        ui.label("Paired:");
                        ui.label(if kb.bluetooth.paired { "Yes" } else { "No" });
                        ui.end_row();

                        if let Some(ref model) = kb.device.model {
                            ui.label("Model:");
                            ui.label(model);
                            ui.end_row();
                        }

                        if let Some(ref name) = kb.device.name {
                            ui.label("Name:");
                            ui.label(name);
                            ui.end_row();
                        }

                        if let Some(ref mac) = kb.device.mac {
                            ui.label("MAC:");
                            ui.label(mac);
                            ui.end_row();
                        }

                        if let Some(ref chip) = kb.device.chip {
                            ui.label("Chip:");
                            ui.label(chip);
                            ui.end_row();
                        }

                        if let Some(ref driver) = kb.device.driver {
                            ui.label("Driver:");
                            ui.label(driver);
                            ui.end_row();
                        }

                        if let Some(ref fw) = kb.firmware.version {
                            ui.label("Firmware:");
                            ui.label(fw);
                            ui.end_row();
                        }

                        if let Some(ref build) = kb.firmware.build {
                            ui.label("Build:");
                            ui.label(build);
                            ui.end_row();
                        }
                    });
            }
        }
    }

    fn tab_display(&mut self, ui: &mut egui::Ui, snap: &SharedState) {
        ui.heading("Display Controls");
        ui.separator();

        if let Some(ref err) = snap.ddc.error {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            // ── Continuous sliders ──────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Image").strong());
                self.vcp_slider(ui, snap, "brightness", 0x10);
                self.vcp_slider(ui, snap, "contrast", 0x12);
                self.vcp_slider(ui, snap, "sharpness", 0x87);
                self.vcp_slider(ui, snap, "volume", 0x62);
                self.vcp_slider(ui, snap, "black_stabilizer", 0xF9);
            });

            ui.add_space(10.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("RGB Gain").strong());
                self.vcp_slider(ui, snap, "red_gain", 0x16);
                self.vcp_slider(ui, snap, "green_gain", 0x18);
                self.vcp_slider(ui, snap, "blue_gain", 0x1A);
            });

            ui.add_space(10.0);

            // ── Picture mode ────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Picture Mode").strong());
                ui.horizontal_wrapped(|ui| {
                    let modes = [
                        (0, "Custom"),
                        (1, "Reader"),
                        (2, "Photo"),
                        (3, "Cinema"),
                        (4, "Color Weakness"),
                        (5, "FPS1"),
                        (6, "FPS2"),
                        (7, "RTS"),
                        (8, "Vivid"),
                        (9, "HDR Effect"),
                        (10, "sRGB"),
                        (11, "DCI-P3"),
                    ];
                    let cur = ddc_cur(snap, "picture_mode");
                    for (val, label) in modes {
                        if ui.selectable_label(cur == val, label).clicked() {
                            self.pending_writes.push((0x15, val));
                        }
                    }
                });
            });

            ui.add_space(10.0);

            // ── Input source ────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Input Source").strong());
                ui.horizontal_wrapped(|ui| {
                    let inputs = [
                        (0x11, "HDMI 1"),
                        (0x12, "HDMI 2"),
                        (0x0F, "DP"),
                        (0x10, "DP Alt"),
                        (0x22, "USB-C"),
                    ];
                    let cur = ddc_cur(snap, "input_source");
                    for (val, label) in inputs {
                        if ui.selectable_label(cur == val, label).clicked() {
                            self.pending_writes.push((0x60, val));
                        }
                    }
                });
            });
        });
    }

    fn tab_advanced(&mut self, ui: &mut egui::Ui, snap: &SharedState) {
        ui.heading("Advanced Settings");
        ui.separator();

        if let Some(ref err) = snap.ddc.error {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Response Time
            self.button_group(ui, snap, "Response Time", "response_time", 0xF7, &[
                (0, "Off"), (1, "High"), (2, "Middle"), (3, "Low"), (4, "Faster"),
            ]);

            // FreeSync
            self.button_group(ui, snap, "FreeSync", "freesync", 0xF8, &[
                (0, "Off"), (1, "Basic"), (2, "Extended"),
            ]);

            // Gamma
            self.button_group(ui, snap, "Gamma", "gamma", 0xFE, &[
                (0, "2.2"), (1, "2.4"), (2, "2.0"), (3, "1.8"),
            ]);

            // Smart Energy
            self.button_group(ui, snap, "Smart Energy Saving", "smart_energy", 0xF6, &[
                (0, "Off"), (1, "High"), (2, "Low"),
            ]);

            // Aspect Ratio
            self.button_group(ui, snap, "Aspect Ratio", "aspect_ratio", 0xF5, &[
                (0, "Full Wide"), (1, "Original"), (2, "Just Scan"), (3, "Cinema 1"),
            ]);

            // Power LED
            self.button_group(ui, snap, "Power LED", "power_led", 0xFD, &[
                (0, "Off"), (1, "On"),
            ]);

            // Split Mode
            self.button_group(ui, snap, "Split / PBP", "split_mode", 0xD7, &[
                (0, "Off"), (1, "PBP"),
            ]);

            // Audio Mute
            self.button_group(ui, snap, "Audio Mute", "audio_mute", 0x8D, &[
                (1, "Muted"), (2, "Unmuted"),
            ]);

            // Language
            self.button_group(ui, snap, "OSD Language", "language", 0xCC, &[
                (0, "English"), (1, "French"), (2, "Deutsch"), (3, "Spanish"),
                (4, "Italian"), (5, "Korean"), (6, "Chinese (S)"), (7, "Japanese"),
                (8, "Portuguese"), (9, "Russian"),
            ]);

            // OSD Lock
            self.button_group(ui, snap, "OSD Lock", "osd_lock", 0xCA, &[
                (1, "Unlocked"), (2, "Locked"),
            ]);
        });
    }

    fn tab_system(&self, ui: &mut egui::Ui, snap: &SharedState) {
        ui.heading("System Info");
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            // ── Monitor ─────────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Monitor").strong());
                egui::Grid::new("sys_monitor")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        if ddc_has(snap, "usage_hours") {
                            ui.label("Usage:");
                            ui.label(format!("{} hours", ddc_cur(snap, "usage_hours")));
                            ui.end_row();
                        }
                        if ddc_has(snap, "backlight_pwm") {
                            ui.label("Backlight PWM:");
                            ui.label(format!("{}", ddc_cur(snap, "backlight_pwm")));
                            ui.end_row();
                        }
                        if ddc_has(snap, "firmware") {
                            let fw = ddc_cur(snap, "firmware");
                            ui.label("Firmware:");
                            ui.label(format!("{}.{}", fw >> 8, fw & 0xFF));
                            ui.end_row();
                        }
                        if ddc_has(snap, "vcp_version") {
                            let ver = ddc_cur(snap, "vcp_version");
                            ui.label("VCP Version:");
                            ui.label(format!("{}.{}", ver >> 8, ver & 0xFF));
                            ui.end_row();
                        }
                        if ddc_has(snap, "display_tech") {
                            ui.label("Display Tech:");
                            ui.label(format!("{}", ddc_cur(snap, "display_tech")));
                            ui.end_row();
                        }
                        if ddc_has(snap, "h_freq") {
                            ui.label("H-Freq:");
                            let hf = ddc_cur(snap, "h_freq");
                            ui.label(format!("{} kHz", hf));
                            ui.end_row();
                        }
                        if ddc_has(snap, "v_freq") {
                            ui.label("V-Freq:");
                            let vf = ddc_cur(snap, "v_freq");
                            ui.label(format!("{}.{} Hz", vf / 100, vf % 100));
                            ui.end_row();
                        }
                        if ddc_has(snap, "power_mode") {
                            let pm = ddc_cur(snap, "power_mode");
                            ui.label("Power Mode:");
                            let label = match pm {
                                1 => "On",
                                2 => "Standby",
                                3 => "Suspend",
                                4 => "Off (soft)",
                                5 => "Off (hard)",
                                _ => "Unknown",
                            };
                            ui.label(label);
                            ui.end_row();
                        }
                        if ddc_has(snap, "color_preset") {
                            ui.label("Color Preset:");
                            ui.label(format!("{}", ddc_cur(snap, "color_preset")));
                            ui.end_row();
                        }
                        if ddc_has(snap, "color_temp_kelvin") {
                            let k = ddc_cur(snap, "color_temp_kelvin");
                            ui.label("Color Temp:");
                            ui.label(format!("{} K", k));
                            ui.end_row();
                        }
                    });
            });

            ui.add_space(10.0);

            // ── Status badges ───────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Status").strong());
                ui.horizontal_wrapped(|ui| {
                    let age = snap.ddc.last_update.map(|t| t.elapsed());
                    let fresh = age.map(|a| a.as_secs() < 10).unwrap_or(false);
                    self.badge(ui, "DDC", fresh);

                    let kb_ok = snap.keyboard.is_some();
                    self.badge(ui, "Keyboard", kb_ok);

                    if let Some(ref kb) = snap.keyboard {
                        self.badge(ui, "BT", kb.bluetooth.connected);
                    }
                });
            });

            ui.add_space(10.0);

            // ── Raw VCP dump ────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Raw VCP Values").strong());
                egui::Grid::new("raw_vcp")
                    .num_columns(3)
                    .spacing([15.0, 2.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("VCP").strong());
                        ui.label(egui::RichText::new("Current").strong());
                        ui.label(egui::RichText::new("Max").strong());
                        ui.end_row();

                        let mut keys: Vec<_> = snap.ddc.data.keys().collect();
                        keys.sort();
                        for k in keys {
                            let (cur, max) = snap.ddc.data[k.as_str()];
                            ui.label(k.as_str());
                            ui.label(format!("{}", cur));
                            ui.label(format!("{}", max));
                            ui.end_row();
                        }
                    });
            });
        });
    }

    // ── Widget helpers ──────────────────────────────────────────────────

    fn vcp_slider(&mut self, ui: &mut egui::Ui, snap: &SharedState, name: &str, vcp: u8) {
        let cur = ddc_cur(snap, name) as f32;
        let max = ddc_max(snap, name) as f32;
        let max_val = if max == 0.0 { 100.0 } else { max };
        let mut val = cur;

        ui.horizontal(|ui| {
            ui.label(format!("{:16}", name));
            let label = format!("{:.0}", val);
            let slider = egui::Slider::new(&mut val, 0.0..=max_val)
                .text(label);
            if ui.add(slider).changed() {
                self.pending_writes.push((vcp, val as u16));
            }
        });
    }

    fn button_group(
        &mut self,
        ui: &mut egui::Ui,
        snap: &SharedState,
        title: &str,
        name: &str,
        vcp: u8,
        options: &[(u16, &str)],
    ) {
        ui.group(|ui| {
            ui.label(egui::RichText::new(title).strong());
            ui.horizontal_wrapped(|ui| {
                let cur = ddc_cur(snap, name);
                for &(val, label) in options {
                    if ui.selectable_label(cur == val, label).clicked() {
                        self.pending_writes.push((vcp, val));
                    }
                }
            });
        });
        ui.add_space(4.0);
    }

    fn badge(&self, ui: &mut egui::Ui, label: &str, ok: bool) {
        let (text, bg) = if ok {
            (format!("{}: OK", label), egui::Color32::from_rgb(30, 100, 40))
        } else {
            (format!("{}: --", label), egui::Color32::from_rgb(100, 30, 30))
        };
        let rt = egui::RichText::new(text)
            .color(egui::Color32::WHITE)
            .strong();
        ui.group(|ui| {
            ui.visuals_mut().widgets.noninteractive.bg_fill = bg;
            ui.label(rt);
        });
    }
}

// ── Entrypoint ──────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ApiHub — Monitor + Keyboard")
            .with_inner_size([720.0, 600.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "apihub",
        options,
        Box::new(|cc| Ok(Box::new(ApiHubApp::new(cc)))),
    )
}
