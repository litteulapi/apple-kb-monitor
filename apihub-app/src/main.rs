mod bluez;
mod brightness;
mod ddc;
mod history;
mod keyboard;
mod mqtt;
mod rssi;

use keyboard::*;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

// ── Profile persistence ────────────────────────────────────────────────────

type DdcProfile = (String, HashMap<String, u16>);

fn profiles_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("apple-kb-monitor/profiles.json")
}

fn load_profiles() -> Vec<DdcProfile> {
    let path = profiles_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str::<Vec<DdcProfile>>(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_profiles(profiles: &[DdcProfile]) {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(profiles) {
        let _ = std::fs::write(&path, json);
    }
}

// ── App Preset persistence ────────────────────────────────────────────────

/// (window_class, picture_mode_value)
type AppPreset = (String, u16);

fn app_presets_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("apple-kb-monitor/app_presets.json")
}

fn load_app_presets() -> Vec<AppPreset> {
    let path = app_presets_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str::<Vec<AppPreset>>(&data).unwrap_or_default(),
        Err(_) => vec![
            ("firefox".to_string(), 15),   // sRGB
            ("steam".to_string(), 30),      // FPS 1
            ("gimp".to_string(), 48),       // Photo
        ],
    }
}

fn save_app_presets(presets: &[AppPreset]) {
    let path = app_presets_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(presets) {
        let _ = std::fs::write(&path, json);
    }
}

/// Get the active window class via xdotool (single justified subprocess).
fn active_window_class() -> Option<String> {
    let output = Command::new("xdotool")
        .args(["getactivewindow", "getwindowclassname"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let class = String::from_utf8_lossy(&output.stdout).trim().to_lowercase();
    if class.is_empty() { None } else { Some(class) }
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

fn apply_burst(state: &State, results: &[(&str, u16, u16)]) {
    if results.is_empty() { return; }
    if let Ok(mut s) = state.lock() {
        for (name, cur, max) in results {
            s.ddc.data.insert(name.to_string(), (*cur, *max));
        }
        s.ddc.last_update = Some(Instant::now());
        s.ddc.error = None;
    }
}

/// Shared app presets (read by poll thread, written by UI).
type SharedPresets = Arc<Mutex<(bool, Vec<AppPreset>)>>;

fn spawn_poll_thread(state: State, presets: SharedPresets) {
    // Start BlueZ Battery Provider (register once, update in loop)
    let battery_provider: Option<bluez::BatteryProvider> = {
        if let Some(kb) = keyboard::read_keyboard() {
            if let Some(ref mac) = kb.device.mac {
                bluez::BatteryProvider::start(mac)
            } else { None }
        } else { None }
    };

    // Start brightness F1/F2 handler
    let bus_for_bri = ddc::default_bus();
    brightness::spawn_brightness_thread(bus_for_bri);

    thread::spawn(move || {
        let bus = ddc::default_bus();
        let mut cycle: u32 = 0;
        let mut battery_notified = false;
        let mut ddc_fail_streak: u32 = 0;
        let mut ddc_fail_notified = false;

        loop {
            // ── Keyboard: direct HID ioctl (~5ms) ──────────────────
            let kb = keyboard::read_keyboard();

            // Battery low notification + BlueZ provider update + history
            if let Some(ref k) = kb {
                let pct = k.battery.percentage_interpolated
                    .or(k.battery.percentage_fine)
                    .or(k.battery.percentage)
                    .unwrap_or(100.0);

                // Update BlueZ Battery Provider (KDE/GNOME battery display)
                if let Some(ref bp) = battery_provider {
                    bp.update_percentage(pct as u8);
                }

                // RSSI from BlueZ MGMT API (pure Rust, no rssi-helper binary)
                if let Some(ref mac) = k.device.mac {
                    if let Some((rssi, tx)) = rssi::read_rssi(mac) {
                        if let Ok(mut s) = state.lock() {
                            if let Some(ref mut kb) = s.keyboard {
                                kb.radio.rssi_dbm = Some(rssi as i32);
                                kb.radio.tx_power_dbm = Some(tx as i32);
                            }
                        }
                    }
                }

                // History logging (every 30th cycle = ~15s)
                if cycle % 30 == 0 {
                    let voltage = k.battery.voltage.unwrap_or(0.0);
                    history::append_history(pct, voltage);
                }

                // Low battery notification
                if !battery_notified && pct < 15.0 {
                    battery_notified = true;
                    let _ = notify_rust::Notification::new()
                        .summary("Apple Keyboard — Low Battery")
                        .body(&format!("Battery at {:.0}% — charge soon", pct))
                        .icon("battery-caution")
                        .show();
                }
            }

            if let Ok(mut s) = state.lock() {
                s.kb_error = if kb.is_none() { Some("Keyboard: not found".into()) } else { None };
                s.keyboard = kb;
            }

            // ── HOT: new_control_value + brightness + volume + backlight ──
            // Every cycle. 0x02 is first — if its value is 0, skip WARM VCPs.
            let hot = ddc::read_batch(&bus, ddc::HOT_VCPS);
            if hot.is_empty() {
                ddc_fail_streak += 1;
                if ddc_fail_streak >= 5 && !ddc_fail_notified {
                    ddc_fail_notified = true;
                    let _ = notify_rust::Notification::new()
                        .summary("DDC/CI Communication Failure")
                        .body("5 consecutive DDC read failures — check I2C bus")
                        .icon("dialog-error")
                        .show();
                }
            } else {
                ddc_fail_streak = 0;
            }
            let new_control_value = hot.iter()
                .find(|(name, _, _)| *name == "new_control_value")
                .map(|(_, cur, _)| *cur)
                .unwrap_or(1); // default to 1 (assume changed) if read failed
            apply_burst(&state, &hot);

            // ── WARM: contrast, RGB, sharpness (~420ms) ────────────
            // Every 4th cycle (~3s) — BUT skip if VCP 0x02 reports no change
            if cycle % 4 == 0 && new_control_value != 0 {
                apply_burst(&state, &ddc::read_batch(&bus, ddc::WARM_VCPS));
            }

            // ── COLD: all settings/info (~1.5s) ────────────────────
            // Every 30th cycle (~22s)
            if cycle % 30 == 0 {
                apply_burst(&state, &ddc::read_batch(&bus, ddc::COLD_VCPS));
            }

            // ── PBP mirror registers (informational) ──────────────
            // Every 30th cycle, only when split mode is active (D7 != 1)
            if cycle % 30 == 0 {
                let split = state.lock().ok()
                    .and_then(|s| s.ddc.data.get("split_mode").map(|v| v.0))
                    .unwrap_or(1);
                if split != 1 {
                    apply_burst(&state, &ddc::read_batch(&bus, ddc::PBP_MIRROR_VCPS));
                }
            }

            // ── App Preset: auto picture mode by active window ────
            // Every 2nd cycle (~1.4s)
            if cycle % 2 == 0 {
                if let Ok(p) = presets.lock() {
                    if p.0 && !p.1.is_empty() {
                        if let Some(wclass) = active_window_class() {
                            for (class, mode) in &p.1 {
                                if wclass.contains(&class.to_lowercase()) {
                                    // Only write if picture mode differs
                                    let current_mode = state.lock().ok()
                                        .and_then(|s| s.ddc.data.get("picture_mode").map(|v| v.0))
                                        .unwrap_or(0);
                                    if current_mode != *mode {
                                        let _ = ddc::ddc_write_vcp(&bus, 0x15, *mode);
                                        if let Ok(mut s) = state.lock() {
                                            let max = s.ddc.data.get("picture_mode").map(|v| v.1).unwrap_or(255);
                                            s.ddc.data.insert("picture_mode".to_string(), (*mode, max));
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            cycle = cycle.wrapping_add(1);

            // HOT cycle: ~210ms I2C + 500ms wait = ~710ms per cycle
            thread::sleep(Duration::from_millis(500));
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
    Mqtt,
    Diag,
}

#[derive(Clone)]
struct MqttConfig {
    broker: String,
    port: String,
    user: String,
    pass: String,
    lamp_entity: String,
    bri_min: f32,
    bri_max: f32,
}

impl MqttConfig {
    fn from_config_file() -> Self {
        let mut cfg = Self {
            broker: String::new(), port: "1883".to_string(),
            user: String::new(), pass: String::new(),
            lamp_entity: "light.bureau".to_string(),
            bri_min: 2.0, bri_max: 70.0,
        };
        // Try ~/.config/apple-kb-monitor/config.toml
        let paths = [
            dirs::config_dir().map(|d| d.join("apple-kb-monitor/config.toml")),
            Some(std::path::PathBuf::from("/etc/apple-kb-monitor/config.toml")),
        ];
        for path in paths.into_iter().flatten() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                // Section-aware TOML parser — only read keys from their proper section
                let mut section = String::new();
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with('[') {
                        section = line.trim_matches(|c| c == '[' || c == ']').trim().to_string();
                        continue;
                    }
                    if line.is_empty() || line.starts_with('#') { continue; }
                    if let Some((key, val)) = line.split_once('=') {
                        let key = key.trim();
                        // Strip inline comments then trim quotes
                        let val = val.trim();
                        let val = if val.starts_with('"') {
                            // Quoted value: find closing quote
                            val.trim_start_matches('"')
                                .splitn(2, '"').next().unwrap_or("")
                        } else {
                            // Unquoted value: strip inline comment
                            val.split('#').next().unwrap_or("").trim()
                        };
                        match (section.as_str(), key) {
                            ("mqtt", "broker") => cfg.broker = val.to_string(),
                            ("mqtt", "port") => cfg.port = val.to_string(),
                            ("mqtt", "user") => cfg.user = val.to_string(),
                            ("mqtt", "password") => cfg.pass = val.to_string(),
                            ("mqtt", "topic_prefix") => {} // recognized but not stored in MqttConfig
                            ("brightness", "min") => cfg.bri_min = val.parse().unwrap_or(2.0),
                            ("brightness", "max") => cfg.bri_max = val.parse().unwrap_or(70.0),
                            ("brightness", "lamp_entity") => cfg.lamp_entity = val.to_string(),
                            _ => {}
                        }
                    }
                }
                eprintln!("[config] loaded {}", path.display());
                break;
            }
        }
        cfg
    }
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self::from_config_file()
    }
}

struct DiagResult {
    label: String,
    ok: bool,
    detail: String,
}

struct ApiHubApp {
    state: State,
    tab: Tab,
    i2c_bus: String,
    pending_writes: Vec<(u8, u16)>,
    style_initialized: bool,
    mqtt: MqttConfig,
    diag_results: Vec<DiagResult>,
    diag_running: bool,
    mqtt_bridge: Option<mqtt::MqttBridge>,
    // DDC profile presets
    profiles: Vec<DdcProfile>,
    profile_name: String,
    // Circadian auto-brightness
    auto_brightness: bool,
    last_auto_bri: u16,
    // App Presets — auto picture mode by active window
    app_presets: Vec<AppPreset>,
    app_presets_enabled: bool,
    app_preset_new_class: String,
    app_preset_new_mode: u16,
    shared_presets: SharedPresets,
    // Factory reset confirmation guard
    confirm_factory_reset: bool,
}

impl ApiHubApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let state: State = Arc::new(Mutex::new(SharedState::default()));
        let app_presets = load_app_presets();
        let shared_presets: SharedPresets = Arc::new(Mutex::new((false, app_presets.clone())));
        spawn_poll_thread(Arc::clone(&state), Arc::clone(&shared_presets));

        let mqtt = MqttConfig::default();
        let i2c_bus = ddc::default_bus();

        // Auto-start MQTT bridge if broker is configured
        let mqtt_bridge = if !mqtt.broker.is_empty() {
            let cfg = mqtt::MqttCfg {
                broker: mqtt.broker.clone(),
                port: mqtt.port.parse().unwrap_or(1883),
                user: mqtt.user.clone(),
                pass: mqtt.pass.clone(),
                topic_prefix: "homeassistant".to_string(),
                monitor_model: "lg_34gn850".to_string(),
                bri_min: mqtt.bri_min as u16,
                bri_max: mqtt.bri_max as u16,
                bus: i2c_bus.clone(),
            };
            eprintln!("[mqtt] auto-start: {}:{}", cfg.broker, cfg.port);
            Some(mqtt::MqttBridge::start(cfg))
        } else {
            None
        };

        Self {
            state,
            tab: Tab::Keyboard,
            i2c_bus,
            pending_writes: Vec::new(),
            style_initialized: false,
            mqtt,
            diag_results: Vec::new(),
            diag_running: false,
            mqtt_bridge,
            profiles: load_profiles(),
            profile_name: String::new(),
            auto_brightness: false,
            last_auto_bri: 0,
            app_presets,
            app_presets_enabled: false,
            app_preset_new_class: String::new(),
            app_preset_new_mode: 15, // default: sRGB
            shared_presets,
            confirm_factory_reset: false,
        }
    }
}

impl eframe::App for ApiHubApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process pending DDC writes — deduplicate per VCP, optimistic UI update
        {
            // Keep only the LAST value per VCP (dedup rapid slider drags)
            let mut deduped: HashMap<u8, u16> = HashMap::new();
            for (vcp, val) in self.pending_writes.drain(..) {
                deduped.insert(vcp, val);
            }
            if !deduped.is_empty() {
                // Optimistic update: immediately reflect new values in UI
                if let Ok(mut s) = self.state.lock() {
                    for (&vcp, &val) in &deduped {
                        for v in ddc::ESSENTIAL_VCPS {
                            if v.code == vcp {
                                let max = s.ddc.data.get(v.name).map(|d| d.1).unwrap_or(255);
                                s.ddc.data.insert(v.name.to_string(), (val, max));
                                break;
                            }
                        }
                    }
                }
                // Single thread for all writes (bus lock serializes anyway)
                let bus = self.i2c_bus.clone();
                thread::spawn(move || {
                    for (vcp, val) in deduped {
                        if let Err(e) = ddc::ddc_write_vcp(&bus, vcp, val) {
                            eprintln!("DDC write 0x{:02X}={}: {}", vcp, val, e);
                        }
                    }
                });
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
            ctx.set_style(style);
            self.style_initialized = true;
        }

        // Request repaint every second for live data
        ctx.request_repaint_after(Duration::from_secs(1));

        let snap = self.state.lock().map(|s| s.clone()).unwrap_or_default();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Keyboard, "Keyboard");
                ui.selectable_value(&mut self.tab, Tab::Display, "Display");
                ui.selectable_value(&mut self.tab, Tab::Advanced, "Advanced");
                ui.selectable_value(&mut self.tab, Tab::System, "System");
                ui.selectable_value(&mut self.tab, Tab::Mqtt, "MQTT");
                ui.selectable_value(&mut self.tab, Tab::Diag, "Diag");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab {
                Tab::Keyboard => self.tab_keyboard(ui, &snap),
                Tab::Display => self.tab_display(ui, &snap),
                Tab::Advanced => self.tab_advanced(ui, &snap),
                Tab::System => self.tab_system(ui, &snap),
                Tab::Mqtt => self.tab_mqtt(ui),
                Tab::Diag => self.tab_diag(ui),
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
        if let Some(ref err) = snap.kb_error {
            ui.label(egui::RichText::new(err.as_str()).size(16.0).color(egui::Color32::from_rgb(255, 100, 100)));
        }

        match &snap.keyboard {
            None => {
                ui.label(egui::RichText::new("Waiting for keyboard data...").size(16.0));
            }
            Some(kb) => {
                let pct = kb.battery.percentage_interpolated
                    .or(kb.battery.percentage_fine)
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
                            ui.add(egui::ProgressBar::new((pct / 100.0).clamp(0.0, 1.0) as f32)
                                .text(format!("{:.1}%", pct)));
                        });
                        ui.add_space(4.0);
                        egui::Grid::new("bat_detail").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                            if let Some(v) = kb.battery.voltage {
                                ui.label(egui::RichText::new("Voltage").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{:.3} V", v)).strong().size(18.0));
                                ui.end_row();
                                ui.label(egui::RichText::new("Type").weak().size(16.0));
                                ui.label(egui::RichText::new(keyboard::detect_battery_type(v)).size(16.0));
                                ui.end_row();
                            }
                            if let Some(adc) = kb.battery.adc_raw {
                                ui.label(egui::RichText::new("ADC").weak().size(16.0));
                                ui.label(egui::RichText::new(format!("{}", adc)).size(16.0));
                                ui.end_row();
                            }
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
                                ui.colored_label(color, egui::RichText::new(format!("{} dBm", r)).strong().size(18.0));
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
                        });
                    });
                });
            }
        }
    }

    fn tab_display(&mut self, ui: &mut egui::Ui, snap: &SharedState) {
        ui.label(egui::RichText::new("Display Controls").strong().size(18.0));
        ui.separator();

        if let Some(ref err) = snap.ddc.error {
            ui.label(egui::RichText::new(err.as_str()).size(16.0).color(egui::Color32::from_rgb(255, 100, 100)));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            // ── Continuous sliders ──────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Image").strong().size(18.0));
                self.vcp_slider(ui, snap, "brightness", 0x10);
                self.vcp_slider(ui, snap, "contrast", 0x12);
                self.vcp_slider(ui, snap, "sharpness", 0x87);
                self.vcp_slider(ui, snap, "black_stabilizer", 0xF9);
            });

            ui.add_space(10.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("Audio").strong().size(18.0));
                self.vcp_slider(ui, snap, "volume", 0x62);
            });

            ui.add_space(10.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("RGB Gain").strong().size(18.0));
                self.vcp_slider(ui, snap, "red_gain", 0x16);
                self.vcp_slider(ui, snap, "green_gain", 0x18);
                self.vcp_slider(ui, snap, "blue_gain", 0x1A);
            });

            ui.add_space(10.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("Video Black Level").strong().size(18.0));
                self.vcp_slider(ui, snap, "black_level_red", 0x6C);
                self.vcp_slider(ui, snap, "black_level_green", 0x6E);
                self.vcp_slider(ui, snap, "black_level_blue", 0x70);
            });

            ui.add_space(10.0);

            // ── Picture mode ────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Picture Mode").strong().size(18.0));
                ui.horizontal_wrapped(|ui| {
                    // Real LG 34GN850 picture mode values (brute-force verified)
                    let modes: [(u16, &str); 14] = [
                        (45, "Custom"),
                        (1, "Reader"),
                        (20, "Vivid"),
                        (22, "HDR Effect"),
                        (46, "Cinema"),
                        (6, "Color Weakness"),
                        (30, "FPS 1"),
                        (31, "FPS 2"),
                        (39, "RTS"),
                        (15, "sRGB"),
                        (24, "DCI-P3"),
                        (25, "EBU"),
                        (48, "Photo"),
                        (49, "Calibration"),
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
                ui.label(egui::RichText::new("Input Source").strong().size(18.0));
                ui.horizontal_wrapped(|ui| {
                    let inputs = [
                        (0x0F, "DisplayPort"),
                        (0x11, "HDMI 1"),
                        (0x12, "HDMI 2"),
                    ];
                    let cur = ddc_cur(snap, "input_source");
                    for (val, label) in inputs {
                        if ui.selectable_label(cur == val, label).clicked() {
                            self.pending_writes.push((0x60, val));
                        }
                    }
                });
            });

            ui.add_space(10.0);

            // ── Auto Brightness (circadian) ────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Auto Brightness").strong().size(18.0));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.auto_brightness, "Enable circadian curve");
                    if self.auto_brightness {
                        let target = brightness::circadian_brightness();
                        ui.label(egui::RichText::new(format!("Target: {}%", target))
                            .strong().size(16.0)
                            .color(egui::Color32::from_rgb(120, 200, 255)));
                        if target != self.last_auto_bri {
                            self.last_auto_bri = target;
                            self.pending_writes.push((0x10, target));
                        }
                    }
                });
            });

            ui.add_space(10.0);

            // ── DDC Profiles ───────────────────────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Profiles").strong().size(18.0));
                ui.add_space(4.0);

                // Save new profile
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Name").weak().size(16.0));
                    ui.add(egui::TextEdit::singleline(&mut self.profile_name).desired_width(140.0));
                    if ui.button(egui::RichText::new("Save").size(16.0)).clicked()
                        && !self.profile_name.trim().is_empty()
                    {
                        let profile_keys: &[(&str, u8)] = &[
                            ("brightness", 0x10), ("contrast", 0x12),
                            ("red_gain", 0x16), ("green_gain", 0x18), ("blue_gain", 0x1A),
                            ("sharpness", 0x87), ("volume", 0x62),
                            ("black_stabilizer", 0xF9), ("picture_mode", 0x15),
                        ];
                        let mut values = HashMap::new();
                        for &(name, _vcp) in profile_keys {
                            if ddc_has(snap, name) {
                                values.insert(name.to_string(), ddc_cur(snap, name));
                            }
                        }
                        let name = self.profile_name.trim().to_string();
                        // Replace if name exists, otherwise append
                        if let Some(pos) = self.profiles.iter().position(|(n, _)| n == &name) {
                            self.profiles[pos].1 = values;
                        } else {
                            self.profiles.push((name, values));
                        }
                        save_profiles(&self.profiles);
                        self.profile_name.clear();
                    }
                });

                ui.add_space(4.0);

                // List saved profiles
                let mut delete_idx: Option<usize> = None;
                for (idx, (name, values)) in self.profiles.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(egui::RichText::new(name).strong().size(16.0)).clicked() {
                            // Restore profile: push all values as pending writes
                            let vcp_map: &[(&str, u8)] = &[
                                ("brightness", 0x10), ("contrast", 0x12),
                                ("red_gain", 0x16), ("green_gain", 0x18), ("blue_gain", 0x1A),
                                ("sharpness", 0x87), ("volume", 0x62),
                                ("black_stabilizer", 0xF9), ("picture_mode", 0x15),
                            ];
                            for &(key, vcp) in vcp_map {
                                if let Some(&val) = values.get(key) {
                                    self.pending_writes.push((vcp, val));
                                }
                            }
                        }
                        if ui.small_button("Delete").clicked() {
                            delete_idx = Some(idx);
                        }
                        // Show summary
                        let summary: Vec<String> = values.iter()
                            .take(4)
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect();
                        let suffix = if values.len() > 4 {
                            format!(" +{}", values.len() - 4)
                        } else {
                            String::new()
                        };
                        ui.label(egui::RichText::new(format!("{}{}", summary.join(", "), suffix))
                            .weak().size(14.0));
                    });
                }
                if let Some(idx) = delete_idx {
                    self.profiles.remove(idx);
                    save_profiles(&self.profiles);
                }
            });

            ui.add_space(10.0);

            // ── App Presets — auto picture mode by active window ───
            ui.group(|ui| {
                ui.label(egui::RichText::new("App Presets").strong().size(18.0));
                ui.add_space(4.0);

                let mut presets_changed = false;
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.app_presets_enabled, "Auto-switch picture mode by window").changed() {
                        presets_changed = true;
                    }
                });

                ui.add_space(4.0);

                // Picture mode lookup for display labels
                let mode_labels: &[(u16, &str)] = &[
                    (45, "Custom"), (1, "Reader"), (20, "Vivid"), (22, "HDR Effect"),
                    (46, "Cinema"), (6, "Color Weakness"), (30, "FPS 1"), (31, "FPS 2"),
                    (39, "RTS"), (15, "sRGB"), (24, "DCI-P3"), (25, "EBU"),
                    (48, "Photo"), (49, "Calibration"),
                ];

                // Existing presets grid
                let mut delete_preset_idx: Option<usize> = None;
                egui::Grid::new("app_presets_grid")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Window Class").strong().size(16.0));
                        ui.label(egui::RichText::new("Picture Mode").strong().size(16.0));
                        ui.label("");
                        ui.end_row();
                        for (idx, (class, mode)) in self.app_presets.iter().enumerate() {
                            ui.label(egui::RichText::new(class).monospace().size(16.0));
                            let label = mode_labels.iter()
                                .find(|(v, _)| v == mode)
                                .map(|(_, l)| *l)
                                .unwrap_or("Unknown");
                            ui.label(egui::RichText::new(format!("{} ({})", label, mode)).size(16.0));
                            if ui.small_button("Delete").clicked() {
                                delete_preset_idx = Some(idx);
                            }
                            ui.end_row();
                        }
                    });

                if let Some(idx) = delete_preset_idx {
                    self.app_presets.remove(idx);
                    save_app_presets(&self.app_presets);
                    presets_changed = true;
                }

                ui.add_space(4.0);

                // Add new preset
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Class").weak().size(16.0));
                    ui.add(egui::TextEdit::singleline(&mut self.app_preset_new_class).desired_width(100.0));
                    ui.label(egui::RichText::new("Mode").weak().size(16.0));
                    egui::ComboBox::from_id_salt("app_preset_mode")
                        .selected_text(
                            mode_labels.iter()
                                .find(|(v, _)| *v == self.app_preset_new_mode)
                                .map(|(_, l)| *l)
                                .unwrap_or("?"),
                        )
                        .show_ui(ui, |ui| {
                            for &(val, label) in mode_labels {
                                ui.selectable_value(&mut self.app_preset_new_mode, val, label);
                            }
                        });
                    if ui.button(egui::RichText::new("Add").size(16.0)).clicked()
                        && !self.app_preset_new_class.trim().is_empty()
                    {
                        let class = self.app_preset_new_class.trim().to_lowercase();
                        // Replace if class exists
                        if let Some(pos) = self.app_presets.iter().position(|(c, _)| c == &class) {
                            self.app_presets[pos].1 = self.app_preset_new_mode;
                        } else {
                            self.app_presets.push((class, self.app_preset_new_mode));
                        }
                        save_app_presets(&self.app_presets);
                        self.app_preset_new_class.clear();
                        presets_changed = true;
                    }
                });

                // Sync to poll thread when anything changes
                if presets_changed {
                    if let Ok(mut p) = self.shared_presets.lock() {
                        p.0 = self.app_presets_enabled;
                        p.1 = self.app_presets.clone();
                    }
                }
            });
        });
    }

    fn tab_advanced(&mut self, ui: &mut egui::Ui, snap: &SharedState) {
        ui.label(egui::RichText::new("Advanced Settings").strong().size(18.0));
        ui.separator();

        if let Some(ref err) = snap.ddc.error {
            ui.label(egui::RichText::new(err.as_str()).size(16.0).color(egui::Color32::from_rgb(255, 100, 100)));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Response Time — brute-force verified: 0-3 accepted, 4 rejected
            self.button_group(ui, snap, "Response Time", "response_time", 0xF7, &[
                (0, "Off"), (1, "Fast"), (2, "Normal"), (3, "Slow"),
            ]);

            // FreeSync — 0/1/2 verified (write causes I2C bus reset, normal)
            self.button_group(ui, snap, "FreeSync", "freesync", 0xF8, &[
                (0, "Off"), (1, "Basic"), (2, "Extended"),
            ]);

            // Gamma via MCCS VCP 0x72 — only 3 values accepted (1.8 rejected)
            self.button_group(ui, snap, "Gamma", "gamma_curve", 0x72, &[
                (0x6400, "2.0"), (0x7800, "2.2"), (0x8C00, "2.4"),
            ]);

            // Smart Energy — only 0 and 2 accepted (1 rejected)
            self.button_group(ui, snap, "Smart Energy Saving", "smart_energy", 0xF6, &[
                (0, "Off"), (2, "High"),
            ]);

            // Aspect Ratio — only 1 accepted on current setup (resolution-dependent)
            self.readonly_group(ui, snap, "Aspect Ratio", "aspect_ratio", &[
                (0, "Full Wide"), (1, "Original"), (2, "Just Scan"),
            ]);

            // Audio Mute
            self.button_group(ui, snap, "Audio Mute", "audio_mute", 0x8D, &[
                (1, "Muted"), (2, "Unmuted"),
            ]);

            // Language — 16 values (0-15), brute-force verified
            self.button_group(ui, snap, "OSD Language", "language", 0xCC, &[
                (0, "EN"), (1, "FR"), (2, "DE"), (3, "ES"),
                (4, "IT"), (5, "KO"), (6, "ZH"), (7, "JA"),
                (8, "PT"), (9, "RU"), (10, "ZH-T"), (11, "PL"),
                (12, "TR"), (13, "CZ"), (14, "SV"), (15, "FI"),
            ]);

            // Read-only VCPs (type=TABLE, DDC writes ignored)
            self.readonly_group(ui, snap, "Power LED", "power_led", &[
                (0, "Off"), (1, "On"),
            ]);

            self.readonly_group(ui, snap, "Split / PBP", "split_mode", &[
                (0, "Off"), (1, "PBP"),
            ]);

            self.readonly_group(ui, snap, "OSD Lock", "osd_lock", &[
                (2, "Unlocked"), (1, "Locked"),
            ]);

            ui.add_space(10.0);

            // ── Maintenance / Factory Reset ───────────────────────────
            ui.group(|ui| {
                ui.label(egui::RichText::new("Maintenance").strong().size(18.0));
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Reset Brightness/Contrast").size(16.0)).clicked() {
                        self.pending_writes.push((0x05, 1));
                    }
                    if ui.button(egui::RichText::new("Reset Color").size(16.0)).clicked() {
                        self.pending_writes.push((0x08, 1));
                    }
                });

                ui.add_space(4.0);

                if self.confirm_factory_reset {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Are you sure? This resets ALL settings.")
                            .size(16.0).color(egui::Color32::from_rgb(255, 70, 70)));
                        if ui.button(egui::RichText::new("Confirm Factory Reset")
                            .size(16.0).strong().color(egui::Color32::from_rgb(255, 70, 70))).clicked()
                        {
                            self.pending_writes.push((0x04, 1));
                            self.confirm_factory_reset = false;
                        }
                        if ui.button(egui::RichText::new("Cancel").size(16.0)).clicked() {
                            self.confirm_factory_reset = false;
                        }
                    });
                } else {
                    if ui.button(egui::RichText::new("Factory Reset ALL").size(16.0)).clicked() {
                        self.confirm_factory_reset = true;
                    }
                }
            });
        });
    }

    fn tab_system(&self, ui: &mut egui::Ui, snap: &SharedState) {
        // Status badges at top, full width
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

        ui.add_space(6.0);

        // Two columns: Monitor info left, Raw VCP right
        ui.columns(2, |cols| {
            // ── LEFT: Monitor info ──────────────────────────────────
            cols[0].group(|ui| {
                ui.label(egui::RichText::new("Monitor").strong().size(18.0));
                ui.add_space(4.0);
                egui::Grid::new("sys_monitor")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        if ddc_has(snap, "power_mode") {
                            let pm = ddc_cur(snap, "power_mode");
                            ui.label(egui::RichText::new("Power").weak().size(16.0));
                            let (label, color) = match pm {
                                1 => ("On", egui::Color32::from_rgb(80, 220, 100)),
                                2 => ("Standby", egui::Color32::from_rgb(255, 200, 50)),
                                3 => ("Suspend", egui::Color32::from_rgb(255, 200, 50)),
                                4 => ("Off (soft)", egui::Color32::from_rgb(255, 70, 70)),
                                5 => ("Off (hard)", egui::Color32::from_rgb(255, 70, 70)),
                                _ => ("Unknown", egui::Color32::GRAY),
                            };
                            ui.label(egui::RichText::new(label).strong().size(18.0).color(color));
                            ui.end_row();
                        }
                        if ddc_has(snap, "usage_hours") {
                            ui.label(egui::RichText::new("Usage").weak().size(16.0));
                            let h = ddc_cur(snap, "usage_hours");
                            ui.label(egui::RichText::new(format!("{} h ({} days)", h, h / 24)).size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "firmware") {
                            let fw = ddc_cur(snap, "firmware");
                            ui.label(egui::RichText::new("Firmware").weak().size(16.0));
                            ui.label(egui::RichText::new(format!("{}.{}", fw >> 8, fw & 0xFF)).strong().size(18.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "vcp_version") {
                            let ver = ddc_cur(snap, "vcp_version");
                            ui.label(egui::RichText::new("VCP Version").weak().size(16.0));
                            ui.label(egui::RichText::new(format!("{}.{}", ver >> 8, ver & 0xFF)).size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "display_tech") {
                            let dt = ddc_cur(snap, "display_tech");
                            ui.label(egui::RichText::new("Panel").weak().size(16.0));
                            let label = match dt {
                                1 => "CRT", 2 => "LCD", 3 => "IPS",
                                4 => "OLED", 5 => "VA", _ => "Unknown",
                            };
                            ui.label(egui::RichText::new(label).size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "backlight_pwm") {
                            let pwm = ddc_cur(snap, "backlight_pwm");
                            let max = ddc_max(snap, "backlight_pwm");
                            ui.label(egui::RichText::new("Backlight").weak().size(16.0));
                            ui.label(egui::RichText::new(format!("{}/{}", pwm, max)).size(16.0));
                            ui.end_row();
                        }
                    });
            });

            cols[0].add_space(8.0);

            cols[0].group(|ui| {
                ui.label(egui::RichText::new("Signal").strong().size(18.0));
                ui.add_space(4.0);
                egui::Grid::new("sys_signal")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        if ddc_has(snap, "h_freq") {
                            let hf = ddc_cur(snap, "h_freq");
                            ui.label(egui::RichText::new("H-Freq").weak().size(16.0));
                            ui.label(egui::RichText::new(format!("{:.2} kHz", hf as f32 / 100.0)).size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "v_freq") {
                            let vf = ddc_cur(snap, "v_freq");
                            ui.label(egui::RichText::new("V-Freq").weak().size(16.0));
                            // LG encoding: raw value, display as-is (non-standard)
                            ui.label(egui::RichText::new(format!("{}", vf)).size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "picture_mode") {
                            let pm = ddc_cur(snap, "picture_mode");
                            ui.label(egui::RichText::new("Picture Mode").weak().size(16.0));
                            let name = match pm {
                                1 => "Reader", 6 => "Color Weakness",
                                15 => "sRGB", 20 => "Vivid", 22 => "HDR Effect",
                                24 => "DCI-P3", 25 => "EBU",
                                30 => "FPS 1", 31 => "FPS 2", 39 => "RTS",
                                45 => "Custom", 46 => "Cinema",
                                48 => "Photo", 49 => "Calibration",
                                _ => "Unknown",
                            };
                            ui.label(egui::RichText::new(format!("{} ({})", name, pm)).strong().size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "color_preset") {
                            let cp = ddc_cur(snap, "color_preset");
                            ui.label(egui::RichText::new("Color Preset").weak().size(16.0));
                            let label = match cp {
                                5 => "6500K", 8 => "9300K", 0x0B => "User", _ => "Other",
                            };
                            ui.label(egui::RichText::new(format!("{} ({})", label, cp)).size(16.0));
                            ui.end_row();
                        }
                        if ddc_has(snap, "color_temp_kelvin") {
                            let k = ddc_cur(snap, "color_temp_kelvin");
                            ui.label(egui::RichText::new("Color Temp").weak().size(16.0));
                            ui.label(egui::RichText::new(format!("{} K", k)).size(16.0));
                            ui.end_row();
                        }
                    });
            });

            // ── PBP Sub-Display (mirror registers) ─────────────────
            // Only shown when split mode is active (D7 != 1)
            {
                let split = ddc_cur(snap, "split_mode");
                let has_mirror = ddc_has(snap, "mirror_brightness")
                    || ddc_has(snap, "mirror_contrast")
                    || ddc_has(snap, "mirror_color_preset");
                if split != 1 && has_mirror {
                    cols[0].add_space(8.0);
                    cols[0].group(|ui| {
                        ui.label(egui::RichText::new("PBP Sub-Display").strong().size(18.0));
                        ui.add_space(4.0);
                        egui::Grid::new("sys_pbp_mirror")
                            .num_columns(2)
                            .spacing([16.0, 8.0])
                            .show(ui, |ui| {
                                if ddc_has(snap, "mirror_brightness") {
                                    ui.label(egui::RichText::new("Brightness").weak().size(16.0));
                                    ui.label(egui::RichText::new(format!("{}", ddc_cur(snap, "mirror_brightness")))
                                        .monospace().size(16.0));
                                    ui.end_row();
                                }
                                if ddc_has(snap, "mirror_contrast") {
                                    ui.label(egui::RichText::new("Contrast").weak().size(16.0));
                                    ui.label(egui::RichText::new(format!("{}", ddc_cur(snap, "mirror_contrast")))
                                        .monospace().size(16.0));
                                    ui.end_row();
                                }
                                if ddc_has(snap, "mirror_color_preset") {
                                    let cp = ddc_cur(snap, "mirror_color_preset");
                                    ui.label(egui::RichText::new("Color Preset").weak().size(16.0));
                                    let label = match cp {
                                        5 => "6500K", 8 => "9300K", 0x0B => "User", _ => "Other",
                                    };
                                    ui.label(egui::RichText::new(format!("{} ({})", label, cp))
                                        .monospace().size(16.0));
                                    ui.end_row();
                                }
                            });
                    });
                }
            }

            // ── RIGHT: Raw VCP dump (scrollable) ────────────────────
            cols[1].group(|ui| {
                ui.label(egui::RichText::new("Raw VCP Values").strong().size(18.0));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(ui.available_height() - 8.0).show(ui, |ui| {
                    egui::Grid::new("raw_vcp")
                        .num_columns(3)
                        .spacing([12.0, 4.0])
                        .striped(true)
                        .min_col_width(60.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("VCP").strong().size(16.0));
                            ui.label(egui::RichText::new("Current").strong().size(16.0));
                            ui.label(egui::RichText::new("Max").strong().size(16.0));
                            ui.end_row();

                            let mut keys: Vec<_> = snap.ddc.data.keys().collect();
                            keys.sort();
                            for k in keys {
                                let (cur, max) = snap.ddc.data[k.as_str()];
                                ui.label(egui::RichText::new(k.as_str()).size(16.0));
                                ui.label(egui::RichText::new(format!("{}", cur)).monospace().size(16.0));
                                ui.label(egui::RichText::new(format!("{}", max)).monospace().size(16.0));
                                ui.end_row();
                            }
                        });
                });
            });
        });
    }

    fn mqtt_cfg(&self) -> mqtt::MqttCfg {
        mqtt::MqttCfg {
            broker: self.mqtt.broker.clone(),
            port: self.mqtt.port.parse().unwrap_or(1883),
            user: self.mqtt.user.clone(),
            pass: self.mqtt.pass.clone(),
            topic_prefix: "homeassistant".to_string(),
            monitor_model: "lg_34gn850".to_string(),
            bri_min: self.mqtt.bri_min as u16,
            bri_max: self.mqtt.bri_max as u16,
            bus: self.i2c_bus.clone(),
        }
    }

    fn tab_mqtt(&mut self, ui: &mut egui::Ui) {
        let bridge_active = self.mqtt_bridge.as_ref()
            .map(|b| b.is_connected()).unwrap_or(false);

        ui.columns(2, |cols| {
            // ── LEFT: Supervision ──────────────────────────────────
            cols[0].group(|ui| {
                ui.label(egui::RichText::new("MQTT Bridge (in-process)").strong().size(18.0));
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Status").weak().size(16.0));
                    if self.mqtt_bridge.is_some() {
                        let (label, color) = if bridge_active {
                            ("Connected", egui::Color32::from_rgb(80, 220, 100))
                        } else {
                            ("Connecting...", egui::Color32::from_rgb(255, 200, 50))
                        };
                        ui.label(egui::RichText::new(label).strong().size(16.0).color(color));
                    } else {
                        ui.label(egui::RichText::new("Stopped").strong().size(16.0)
                            .color(egui::Color32::from_rgb(255, 70, 70)));
                    }
                });

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if self.mqtt_bridge.is_some() {
                        if ui.button(egui::RichText::new("Stop").size(16.0)).clicked() {
                            self.mqtt_bridge = None;
                        }
                    } else {
                        if ui.button(egui::RichText::new("Start").size(16.0)).clicked() {
                            if !self.mqtt.broker.is_empty() {
                                self.mqtt_bridge = Some(mqtt::MqttBridge::start(self.mqtt_cfg()));
                            }
                        }
                    }
                    if ui.button(egui::RichText::new("Publish Now").size(16.0)).clicked() {
                        if let Some(bridge) = &self.mqtt_bridge {
                            let snap = self.state.lock().map(|s| s.clone()).unwrap_or_default();
                            bridge.publish_telemetry(&snap.keyboard, &snap.ddc.data, &self.mqtt_cfg());
                        }
                    }
                });

                // Last command received
                if let Some(bridge) = &self.mqtt_bridge {
                    if let Ok(lc) = bridge.last_cmd.lock() {
                        if let Some(cmd) = lc.as_ref() {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("Last: {}", cmd)).weak().size(16.0));
                        }
                    }
                    if let Ok(lp) = bridge.last_publish.lock() {
                        if let Some(t) = lp.as_ref() {
                            let ago = t.elapsed().as_secs();
                            ui.label(egui::RichText::new(format!("Published {}s ago", ago)).weak().size(16.0));
                        }
                    }
                }
            });

            cols[0].add_space(8.0);

            cols[0].group(|ui| {
                ui.label(egui::RichText::new("Lamp → Monitor Sync").strong().size(18.0));
                ui.add_space(4.0);
                egui::Grid::new("mqtt_sync")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Source").weak().size(16.0));
                        ui.label(egui::RichText::new(&self.mqtt.lamp_entity).monospace().size(16.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Formula").weak().size(16.0));
                        ui.label(egui::RichText::new(format!(
                            "{} + (lamp/255) × {}", self.mqtt.bri_min as u16,
                            (self.mqtt.bri_max - self.mqtt.bri_min) as u16
                        )).monospace().size(16.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Range").weak().size(16.0));
                        ui.label(egui::RichText::new(format!(
                            "{}% – {}%", self.mqtt.bri_min as u16, self.mqtt.bri_max as u16
                        )).strong().size(16.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Broker").weak().size(16.0));
                        ui.label(egui::RichText::new(format!(
                            "{}:{}", self.mqtt.broker, self.mqtt.port
                        )).monospace().size(16.0));
                        ui.end_row();
                    });
            });

            // ── RIGHT: Settings ────────────────────────────────────
            cols[1].group(|ui| {
                ui.label(egui::RichText::new("Settings").strong().size(18.0));
                ui.add_space(4.0);
                egui::Grid::new("mqtt_settings")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Broker").weak().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut self.mqtt.broker).desired_width(180.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Port").weak().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut self.mqtt.port).desired_width(60.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("User").weak().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut self.mqtt.user).desired_width(180.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Password").weak().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut self.mqtt.pass).password(true).desired_width(180.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Lamp Entity").weak().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut self.mqtt.lamp_entity).desired_width(180.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("I2C Bus").weak().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut self.i2c_bus).desired_width(180.0));
                        ui.end_row();
                    });

                ui.add_space(12.0);
                ui.label(egui::RichText::new("Brightness Range").strong().size(18.0));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Min").weak().size(16.0));
                    ui.add(egui::Slider::new(&mut self.mqtt.bri_min, 0.0..=30.0).show_value(true));
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Max").weak().size(16.0));
                    ui.add(egui::Slider::new(&mut self.mqtt.bri_max, 30.0..=100.0).show_value(true));
                });

                ui.add_space(12.0);
                if ui.button(egui::RichText::new("Save Config & Reconnect").size(16.0).strong()).clicked() {
                    let config_dir = dirs::config_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                        .join("apple-kb-monitor");
                    let _ = std::fs::create_dir_all(&config_dir);
                    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
                    let toml = format!(
                        "[ddc]\nbus = \"{}\"\n\n[mqtt]\nbroker = \"{}\"\nport = {}\nuser = \"{}\"\npassword = \"{}\"\ntopic_prefix = \"homeassistant\"\n\n[monitor]\nmodel = \"lg_34gn850\"\n\n[brightness]\nmin = {}\nmax = {}\nlamp_entity = \"{}\"\n",
                        esc(&self.i2c_bus), esc(&self.mqtt.broker), self.mqtt.port,
                        esc(&self.mqtt.user), esc(&self.mqtt.pass),
                        self.mqtt.bri_min as u16, self.mqtt.bri_max as u16,
                        esc(&self.mqtt.lamp_entity),
                    );
                    let _ = std::fs::write(config_dir.join("config.toml"), &toml);
                    // Restart in-process bridge with new config
                    self.mqtt_bridge = Some(mqtt::MqttBridge::start(self.mqtt_cfg()));
                }
            });
        });
    }

    fn run_diagnostics(&mut self) {
        self.diag_results.clear();
        self.diag_running = true;

        // Extract bus number from path (e.g. "/dev/i2c-6" -> "6")
        let bus_num = self.i2c_bus.rsplit('-').next().unwrap_or("6").to_string();
        let checks: Vec<(&str, Vec<&str>, &str)> = vec![
            ("apple-kb-monitor", vec!["--version"], "Main daemon binary"),
            ("ddc-tool", vec!["read", &bus_num, "0x10"], "DDC/CI I2C tool"),
            ("keyd", vec!["-v"], "Key remapping daemon"),
            ("bluetoothctl", vec!["--version"], "BlueZ CLI"),
            ("mosquitto_pub", vec!["--help"], "MQTT publish tool"),
        ];

        for (bin, args, desc) in &checks {
            let result = Command::new(bin).args(args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();
            match result {
                Ok(o) if o.status.success() => {
                    let out = String::from_utf8_lossy(&o.stdout);
                    let first = out.lines().next().unwrap_or("OK").trim();
                    self.diag_results.push(DiagResult {
                        label: desc.to_string(), ok: true,
                        detail: format!("{}: {}", bin, if first.is_empty() { "OK" } else { first }),
                    });
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    // Some tools return non-zero for --help but still work
                    if err.contains("Usage") || err.contains("usage") || err.contains("mosquitto_pub") {
                        self.diag_results.push(DiagResult {
                            label: desc.to_string(), ok: true,
                            detail: format!("{}: installed", bin),
                        });
                    } else {
                        self.diag_results.push(DiagResult {
                            label: desc.to_string(), ok: false,
                            detail: format!("{}: exit {}", bin, o.status.code().unwrap_or(-1)),
                        });
                    }
                }
                Err(_) => {
                    self.diag_results.push(DiagResult {
                        label: desc.to_string(), ok: false,
                        detail: format!("{}: NOT FOUND", bin),
                    });
                }
            }
        }

        // Services
        for svc in ["apple-kb-monitor", "apple-brightness", "mqtt-bridge"] {
            let result = Command::new("systemctl")
                .args(["--user", "is-active", &format!("{}.service", svc)])
                .output();
            let active = result.map(|o| o.status.success()).unwrap_or(false);
            self.diag_results.push(DiagResult {
                label: format!("{}.service", svc), ok: active,
                detail: if active { "active (running)".into() } else { "inactive / not found".into() },
            });
        }

        // I2C bus
        let bus = &self.i2c_bus;
        let i2c_ok = std::path::Path::new(bus).exists();
        self.diag_results.push(DiagResult {
            label: "I2C bus".into(), ok: i2c_ok,
            detail: if i2c_ok { format!("{}: accessible", bus) } else { format!("{}: NOT FOUND", bus) },
        });

        // hidraw (keyboard) — enumerate /dev/hidraw*
        let hidraw_count = std::fs::read_dir("/dev").map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("hidraw"))
                .count()
        }).unwrap_or(0);
        self.diag_results.push(DiagResult {
            label: "HID raw device".into(), ok: hidraw_count > 0,
            detail: if hidraw_count > 0 { format!("{} hidraw device(s) in /dev/", hidraw_count) }
                    else { "no hidraw device found".into() },
        });

        // Config file
        let cfg_path = dirs::config_dir()
            .map(|d| d.join("apple-kb-monitor/config.toml"))
            .unwrap_or_default();
        let cfg_ok = cfg_path.exists();
        self.diag_results.push(DiagResult {
            label: "Config file".into(), ok: cfg_ok,
            detail: if cfg_ok { format!("{}", cfg_path.display()) } else { "not found — copy config.toml.example".into() },
        });

        // keyd config
        let keyd_ok = std::path::Path::new("/etc/keyd/apple-keyboard.conf").exists();
        self.diag_results.push(DiagResult {
            label: "keyd config".into(), ok: keyd_ok,
            detail: if keyd_ok { "/etc/keyd/apple-keyboard.conf".into() } else { "NOT FOUND".into() },
        });

        // udev rules
        let udev_ok = std::path::Path::new("/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules").exists();
        self.diag_results.push(DiagResult {
            label: "udev rules".into(), ok: udev_ok,
            detail: if udev_ok { "99-apple-kb-hidraw.rules installed".into() } else { "NOT FOUND".into() },
        });

        // modprobe
        let mod_ok = std::path::Path::new("/etc/modprobe.d/hid_apple.conf").exists();
        self.diag_results.push(DiagResult {
            label: "hid_apple fnmode".into(), ok: mod_ok,
            detail: if mod_ok { "fnmode=1 configured".into() } else { "NOT FOUND — media keys won't be default".into() },
        });

        // rssi-helper caps
        let rssi_path = std::path::Path::new("/usr/lib/apple-kb-monitor/rssi-helper");
        let rssi_ok = rssi_path.exists();
        self.diag_results.push(DiagResult {
            label: "RSSI helper".into(), ok: rssi_ok,
            detail: if rssi_ok { "rssi-helper installed (needs CAP_NET_ADMIN)".into() } else { "NOT FOUND".into() },
        });

        // user in input group (pure libc — no subprocess)
        let input_ok = {
            let mut buf = [0i32; 64];
            let n = unsafe { libc::getgroups(64, buf.as_mut_ptr() as *mut u32) };
            if n > 0 {
                let input_content = std::fs::read_to_string("/etc/group").unwrap_or_default();
                let input_gid = input_content.lines()
                    .find(|l| l.starts_with("input:"))
                    .and_then(|l| l.split(':').nth(2))
                    .and_then(|s| s.parse::<i32>().ok());
                input_gid.map(|gid| buf[..n as usize].contains(&gid)).unwrap_or(false)
            } else {
                false
            }
        };
        self.diag_results.push(DiagResult {
            label: "User in input group".into(), ok: input_ok,
            detail: if input_ok { "input group: OK".into() } else { "NOT in input group — run: sudo usermod -aG input $USER".into() },
        });

        self.diag_running = false;
    }

    fn tab_diag(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("System Diagnostics").strong().size(18.0));
            if ui.button(egui::RichText::new("Run Full Check").size(16.0).strong()).clicked() {
                self.run_diagnostics();
            }
        });
        ui.separator();

        if self.diag_results.is_empty() {
            ui.label(egui::RichText::new("Press 'Run Full Check' to scan all components.").weak().size(16.0));
            return;
        }

        let total = self.diag_results.len();
        let ok_count = self.diag_results.iter().filter(|r| r.ok).count();
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
            for r in &self.diag_results {
                ui.horizontal(|ui| {
                    let (icon, color) = if r.ok {
                        ("OK", egui::Color32::from_rgb(80, 220, 100))
                    } else {
                        ("FAIL", egui::Color32::from_rgb(255, 70, 70))
                    };
                    ui.label(egui::RichText::new(icon).strong().size(16.0).color(color)
                        .background_color(if r.ok { egui::Color32::from_rgb(20, 50, 20) }
                                         else { egui::Color32::from_rgb(60, 20, 20) }));
                    ui.label(egui::RichText::new(&r.label).strong().size(16.0));
                    ui.label(egui::RichText::new(&r.detail).weak().size(16.0));
                });
            }
        });
    }

    // ── Widget helpers ──────────────────────────────────────────────────

    fn vcp_slider(&mut self, ui: &mut egui::Ui, snap: &SharedState, name: &str, vcp: u8) {
        let cur = ddc_cur(snap, name) as f32;
        let max = ddc_max(snap, name) as f32;
        let max_val = if max == 0.0 { 100.0 } else { max };
        let mut val = cur;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).monospace().size(16.0));
            let slider = egui::Slider::new(&mut val, 0.0..=max_val)
                .show_value(true);
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
            ui.label(egui::RichText::new(title).strong().size(18.0));
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

    fn readonly_group(
        &self,
        ui: &mut egui::Ui,
        snap: &SharedState,
        title: &str,
        name: &str,
        options: &[(u16, &str)],
    ) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).strong().size(18.0));
                if !ddc_has(snap, name) {
                    ui.label(egui::RichText::new("loading...").weak().size(16.0).italics());
                }
            });
            if ddc_has(snap, name) {
                ui.horizontal_wrapped(|ui| {
                    let cur = ddc_cur(snap, name);
                    for &(val, label) in options {
                        let selected = cur == val;
                        let text = if selected {
                            egui::RichText::new(format!("  {}  ", label)).strong().size(16.0)
                                .color(egui::Color32::from_rgb(120, 200, 255))
                                .background_color(egui::Color32::from_rgb(30, 50, 70))
                        } else {
                            egui::RichText::new(format!("  {}  ", label)).weak().size(16.0)
                        };
                        ui.label(text);
                    }
                });
            }
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
            .strong()
            .size(16.0);
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
