//! In-process MQTT client for Home Assistant integration.
//!
//! Publishes keyboard + monitor telemetry, subscribes to brightness commands.
//! Zero subprocess — uses rumqttc async client running in a dedicated thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rumqttc::{Client, Event, Incoming, MqttOptions, QoS};

use crate::ddc;

/// Picture-mode names exposed to Home Assistant, with their DDC values (VCP 0x15).
const PICTURE_MODES: &[(&str, u16)] = &[
    ("Custom", 45), ("Reader", 1), ("Vivid", 20), ("HDR Effect", 22), ("Cinema", 46),
    ("Color Weakness", 6), ("FPS 1", 30), ("FPS 2", 31), ("RTS", 39), ("sRGB", 15),
    ("DCI-P3", 24), ("EBU", 25), ("Photo", 48), ("Calibration", 49),
];

/// Input sources exposed to Home Assistant, with their DDC values (VCP 0x60).
const INPUT_SOURCES: &[(&str, u16)] = &[("DisplayPort", 0x0F), ("HDMI 1", 0x11), ("HDMI 2", 0x12)];

fn picture_mode_value(name: &str) -> Option<u16> {
    PICTURE_MODES.iter().find(|(n, _)| *n == name).map(|&(_, v)| v)
}

/// `None` for values that are not a select option (HA rejects unknown states).
fn picture_mode_name(value: u16) -> Option<&'static str> {
    PICTURE_MODES.iter().find(|(_, v)| *v == value).map(|&(n, _)| n)
}

fn input_value(name: &str) -> Option<u16> {
    INPUT_SOURCES.iter().find(|(n, _)| *n == name).map(|&(_, v)| v)
}

fn input_name(value: u16) -> Option<&'static str> {
    INPUT_SOURCES.iter().find(|(_, v)| *v == value).map(|&(n, _)| n)
}

/// Parse an HA "number" payload ("42", "42.0") into a rounded, non-negative value.
/// Rejects NaN/inf and saturates negatives to 0 and huge values to u16::MAX.
fn parse_percent(payload: &str) -> Option<u16> {
    let v: f32 = payload.trim().parse().ok()?;
    if !v.is_finite() { return None; }
    Some(v.round() as u16)
}

/// Clamp into [min, max] without panicking when the config has min > max.
fn clamp_range(v: u16, min: u16, max: u16) -> u16 {
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    v.clamp(lo, hi)
}

/// MQTT bridge state shared with the UI thread.
pub struct MqttBridge {
    pub connected: Arc<Mutex<bool>>,
    pub last_publish: Arc<Mutex<Option<std::time::Instant>>>,
    pub last_cmd: Arc<Mutex<Option<String>>>,
    tx: Option<rumqttc::Client>,
}

#[derive(Clone)]
pub struct MqttCfg {
    pub broker: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub topic_prefix: String,
    pub monitor_model: String,
    pub bri_min: u16,
    pub bri_max: u16,
    pub bus: String,
}

impl MqttBridge {
    /// Start the MQTT bridge in a background thread.
    /// Returns immediately — connection happens async.
    pub fn start(cfg: MqttCfg) -> Self {
        let connected = Arc::new(Mutex::new(false));
        let last_publish = Arc::new(Mutex::new(None));
        let last_cmd = Arc::new(Mutex::new(None));

        let conn = connected.clone();
        let lc = last_cmd.clone();
        let lp = last_publish.clone();
        let lc_thread = lc.clone();

        let mut opts = MqttOptions::new("apihub-app", &cfg.broker, cfg.port);
        opts.set_keep_alive(Duration::from_secs(30));
        if !cfg.user.is_empty() {
            opts.set_credentials(&cfg.user, &cfg.pass);
        }

        let (client, mut connection) = Client::new(opts, 64);

        // Subscribe to brightness command topic
        let cmd_brightness = format!("{}/number/{}/brightness/set", cfg.topic_prefix, cfg.monitor_model);
        let cmd_volume = format!("{}/number/{}/volume/set", cfg.topic_prefix, cfg.monitor_model);
        let cmd_picture = format!("{}/select/{}/picture_mode/set", cfg.topic_prefix, cfg.monitor_model);
        let cmd_input = format!("{}/select/{}/input_source/set", cfg.topic_prefix, cfg.monitor_model);
        let _ = client.try_subscribe(&cmd_brightness, QoS::AtMostOnce);
        let _ = client.try_subscribe(&cmd_volume, QoS::AtMostOnce);
        let _ = client.try_subscribe(&cmd_picture, QoS::AtMostOnce);
        let _ = client.try_subscribe(&cmd_input, QoS::AtMostOnce);

        let tx = client.clone();

        // Spawn event loop thread (blocking iterator over Connection)
        thread::spawn(move || {
            for notification in connection.iter() {
                match notification {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        if let Ok(mut c) = conn.lock() { *c = true; }
                        let _ = client.try_subscribe(&cmd_brightness, QoS::AtMostOnce);
                        let _ = client.try_subscribe(&cmd_volume, QoS::AtMostOnce);
                        let _ = client.try_subscribe(&cmd_picture, QoS::AtMostOnce);
                        let _ = client.try_subscribe(&cmd_input, QoS::AtMostOnce);
                        eprintln!("[mqtt] connected to {}:{}", cfg.broker, cfg.port);
                    }
                    Ok(Event::Incoming(Incoming::Publish(msg))) => {
                        let payload_str = std::str::from_utf8(&msg.payload).unwrap_or("");
                        let bus = &cfg.bus;
                        let prefix = &cfg.topic_prefix;
                        let model = &cfg.monitor_model;

                        let trimmed = payload_str.trim();
                        // (vcp, value, state subtopic, state payload, log label)
                        let action: Option<(u8, u16, String, String, String)> = if msg.topic == cmd_brightness {
                            parse_percent(trimmed).map(|v| {
                                let v = clamp_range(v, cfg.bri_min, cfg.bri_max);
                                (0x10, v, format!("{}/number/{}/brightness/state", prefix, model),
                                 v.to_string(), format!("brightness → {}", v))
                            })
                        } else if msg.topic == cmd_volume {
                            parse_percent(trimmed).map(|v| {
                                let v = v.min(100);
                                (0x62, v, format!("{}/number/{}/volume/state", prefix, model),
                                 v.to_string(), format!("volume → {}", v))
                            })
                        } else if msg.topic == cmd_picture {
                            picture_mode_value(trimmed).map(|v| {
                                (0x15, v, format!("{}/select/{}/picture_mode/state", prefix, model),
                                 trimmed.to_string(), format!("mode → {}", trimmed))
                            })
                        } else if msg.topic == cmd_input {
                            input_value(trimmed).map(|v| {
                                (0x60, v, format!("{}/select/{}/input_source/state", prefix, model),
                                 trimmed.to_string(), format!("input → {}", trimmed))
                            })
                        } else {
                            None
                        };

                        if let Some((vcp, v, state_topic, state, label)) = action {
                            // Acknowledge only what was really applied: echoing the state
                            // after a failed DDC write would show a wrong value in HA.
                            match ddc::ddc_write_vcp(bus, vcp, v) {
                                Ok(()) => {
                                    let _ = client.try_publish(state_topic, QoS::AtMostOnce, true, state.as_bytes());
                                    if let Ok(mut l) = lc_thread.lock() { *l = Some(label); }
                                }
                                Err(e) => {
                                    eprintln!("[mqtt] DDC write 0x{:02X}={} failed: {}", vcp, v, e);
                                    if let Ok(mut l) = lc_thread.lock() { *l = Some(format!("{} (FAILED)", label)); }
                                }
                            }
                        }
                    }
                    Ok(Event::Incoming(Incoming::Disconnect)) => {
                        if let Ok(mut c) = conn.lock() { *c = false; }
                        eprintln!("[mqtt] disconnected");
                    }
                    Err(e) => {
                        if let Ok(mut c) = conn.lock() { *c = false; }
                        eprintln!("[mqtt] error: {}", e);
                        thread::sleep(Duration::from_secs(5));
                    }
                    _ => {}
                }
            }
        });

        Self {
            connected,
            last_publish: lp,
            last_cmd: lc,
            tx: Some(tx),
        }
    }

    /// Publish all keyboard + monitor data to HA auto-discovery.
    pub fn publish_telemetry(
        &self,
        kb: &Option<crate::keyboard::KbReport>,
        ddc_data: &HashMap<String, (u16, u16)>,
        cfg: &MqttCfg,
    ) {
        let tx = match &self.tx {
            Some(c) => c,
            None => return,
        };

        let prefix = &cfg.topic_prefix;
        let model = &cfg.monitor_model;

        // ── Keyboard sensors ───────────────────────────────────────
        if let Some(kb) = kb {
            let mac = kb.device.mac.as_deref().unwrap_or("unknown").replace(":", "").to_lowercase();
            let dev_name = kb.device.model.as_deref().unwrap_or("Apple Keyboard");
            let fw = kb.firmware.version.as_deref().unwrap_or("");

            let device = format!(
                r#"{{"identifiers":["apple_kb_{}"],"name":"{}","manufacturer":"Apple","model":"{}","sw_version":"{}"}}"#,
                mac, dev_name, dev_name, fw
            );

            let pct = kb.battery.percentage_fine
                .or(kb.battery.percentage_interpolated)
                .or(kb.battery.percentage);

            let sensors: Vec<(&str, Option<String>, &str, &str, Option<&str>)> = vec![
                ("battery", pct.map(|v| format!("{:.0}", v)), "%", "mdi:battery-bluetooth", Some("battery")),
                ("voltage", kb.battery.voltage.map(|v| format!("{:.3}", v)), "V", "mdi:flash-triangle", None),
                ("rssi", kb.radio.rssi_dbm.map(|v| format!("{}", v)), "dBm", "mdi:bluetooth", Some("signal_strength")),
                ("tx_power", kb.radio.tx_power_dbm.map(|v| format!("{}", v)), "dBm", "mdi:access-point", None),
            ];

            // Connected binary sensor
            let connected_config = format!(
                r#"{{"name":"Apple KB Connected","unique_id":"apple_kb_{}_connected","state_topic":"{}/binary_sensor/apple_kb_{}/connected/state","device_class":"connectivity","icon":"mdi:keyboard-wireless","device":{}}}"#,
                mac, prefix, mac, device
            );
            let _ = tx.try_publish(
                format!("{}/binary_sensor/apple_kb_{}/connected/config", prefix, mac),
                QoS::AtMostOnce, true, connected_config.as_bytes(),
            );
            let _ = tx.try_publish(
                format!("{}/binary_sensor/apple_kb_{}/connected/state", prefix, mac),
                QoS::AtMostOnce, true,
                if kb.bluetooth.connected { b"ON" as &[u8] } else { b"OFF" },
            );

            for (sid, val, unit, icon, dev_class) in &sensors {
                if let Some(val) = val {
                    let config = if let Some(dc) = dev_class {
                        format!(
                            r#"{{"name":"Apple KB {}","unique_id":"apple_kb_{}_{}","state_topic":"{}/sensor/apple_kb_{}/{}/state","unit_of_measurement":"{}","icon":"{}","device_class":"{}","device":{}}}"#,
                            sid, mac, sid, prefix, mac, sid, unit, icon, dc, device
                        )
                    } else {
                        format!(
                            r#"{{"name":"Apple KB {}","unique_id":"apple_kb_{}_{}","state_topic":"{}/sensor/apple_kb_{}/{}/state","unit_of_measurement":"{}","icon":"{}","device":{}}}"#,
                            sid, mac, sid, prefix, mac, sid, unit, icon, device
                        )
                    };
                    let _ = tx.try_publish(
                        format!("{}/sensor/apple_kb_{}/{}/config", prefix, mac, sid),
                        QoS::AtMostOnce, true, config.as_bytes(),
                    );
                    let _ = tx.try_publish(
                        format!("{}/sensor/apple_kb_{}/{}/state", prefix, mac, sid),
                        QoS::AtMostOnce, true, val.as_bytes(),
                    );
                }
            }
        }

        // ── Monitor sensors ────────────────────────────────────────
        let device_mon = format!(
            r#"{{"identifiers":["{}"],"name":"LG 34GN850","manufacturer":"LG Electronics","model":"34GN850"}}"#,
            model
        );

        let mon_sensors: Vec<(&str, &str, &str)> = vec![
            ("brightness", "%", "mdi:brightness-6"),
            ("contrast", "%", "mdi:contrast-box"),
            ("volume", "%", "mdi:volume-high"),
            ("color_temp_kelvin", "K", "mdi:thermometer"),
            ("usage_hours", "h", "mdi:clock-outline"),
            ("backlight_pwm", "", "mdi:lightbulb"),
        ];

        for (sid, unit, icon) in &mon_sensors {
            if let Some((cur, _)) = ddc_data.get(*sid) {
                let config = format!(
                    r#"{{"name":"LG {}","unique_id":"{}_{}","state_topic":"{}/sensor/{}/{}/state","unit_of_measurement":"{}","icon":"{}","device":{}}}"#,
                    sid.replace('_', " "), model, sid, prefix, model, sid, unit, icon, device_mon
                );
                let _ = tx.try_publish(
                    format!("{}/sensor/{}/{}/config", prefix, model, sid),
                    QoS::AtMostOnce, true, config.as_bytes(),
                );
                let _ = tx.try_publish(
                    format!("{}/sensor/{}/{}/state", prefix, model, sid),
                    QoS::AtMostOnce, true, cur.to_string().as_bytes(),
                );
            }
        }

        // ── Brightness number entity ───────────────────────────────
        let num_config = format!(
            r#"{{"name":"LG Monitor Brightness","unique_id":"{}_brightness_ctrl","command_topic":"{}/number/{}/brightness/set","state_topic":"{}/number/{}/brightness/state","min":{},"max":{},"step":1,"unit_of_measurement":"%","icon":"mdi:monitor-shimmer","device":{}}}"#,
            model, prefix, model, prefix, model, cfg.bri_min, cfg.bri_max, device_mon
        );
        let _ = tx.try_publish(
            format!("{}/number/{}/brightness/config", prefix, model),
            QoS::AtMostOnce, true, num_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("brightness") {
            let _ = tx.try_publish(
                format!("{}/number/{}/brightness/state", prefix, model),
                QoS::AtMostOnce, true, cur.to_string().as_bytes(),
            );
        }

        // ── Volume number entity ──────────────────────────────────
        let vol_config = format!(
            r#"{{"name":"LG Monitor Volume","unique_id":"{}_volume_ctrl","command_topic":"{}/number/{}/volume/set","state_topic":"{}/number/{}/volume/state","min":0,"max":100,"step":1,"unit_of_measurement":"%","icon":"mdi:volume-high","device":{}}}"#,
            model, prefix, model, prefix, model, device_mon
        );
        let _ = tx.try_publish(
            format!("{}/number/{}/volume/config", prefix, model),
            QoS::AtMostOnce, true, vol_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("volume") {
            let _ = tx.try_publish(
                format!("{}/number/{}/volume/state", prefix, model),
                QoS::AtMostOnce, true, cur.to_string().as_bytes(),
            );
        }

        // ── Picture mode select entity ─────────────────────────────
        let modes = "Custom,Reader,Vivid,HDR Effect,Cinema,Color Weakness,FPS 1,FPS 2,RTS,sRGB,DCI-P3,EBU,Photo,Calibration";
        let pm_config = format!(
            r#"{{"name":"LG Picture Mode","unique_id":"{}_picture_mode","command_topic":"{}/select/{}/picture_mode/set","state_topic":"{}/select/{}/picture_mode/state","options":[{}],"icon":"mdi:image-filter-hdr","device":{}}}"#,
            model, prefix, model, prefix, model,
            modes.split(',').map(|m| format!("\"{}\"", m)).collect::<Vec<_>>().join(","),
            device_mon
        );
        let _ = tx.try_publish(
            format!("{}/select/{}/picture_mode/config", prefix, model),
            QoS::AtMostOnce, true, pm_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("picture_mode") {
            if let Some(name) = picture_mode_name(*cur) {
                let _ = tx.try_publish(
                    format!("{}/select/{}/picture_mode/state", prefix, model),
                    QoS::AtMostOnce, true, name.as_bytes(),
                );
            }
        }

        // ── Input source select entity ─────────────────────────────
        let is_config = format!(
            r#"{{"name":"LG Input Source","unique_id":"{}_input_source","command_topic":"{}/select/{}/input_source/set","state_topic":"{}/select/{}/input_source/state","options":["DisplayPort","HDMI 1","HDMI 2"],"icon":"mdi:video-input-hdmi","device":{}}}"#,
            model, prefix, model, prefix, model, device_mon
        );
        let _ = tx.try_publish(
            format!("{}/select/{}/input_source/config", prefix, model),
            QoS::AtMostOnce, true, is_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("input_source") {
            if let Some(name) = input_name(*cur) {
                let _ = tx.try_publish(
                    format!("{}/select/{}/input_source/state", prefix, model),
                    QoS::AtMostOnce, true, name.as_bytes(),
                );
            }
        }

        if let Ok(mut lp) = self.last_publish.lock() {
            *lp = Some(std::time::Instant::now());
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.lock().map(|c| *c).unwrap_or(false)
    }

    /// Clone the internal MQTT client handle for fire-and-forget publishes.
    pub fn tx_clone(&self) -> Option<Client> {
        self.tx.clone()
    }

    /// Reconstruct a bridge handle from pre-existing shared parts.
    /// Used for fire-and-forget publishes on a background thread (M11).
    pub fn from_parts(
        connected: Arc<Mutex<bool>>,
        last_publish: Arc<Mutex<Option<std::time::Instant>>>,
        last_cmd: Arc<Mutex<Option<String>>>,
        tx: Client,
    ) -> Self {
        Self { connected, last_publish, last_cmd, tx: Some(tx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_parsing() {
        assert_eq!(parse_percent("42"), Some(42));
        assert_eq!(parse_percent(" 42.6 \n"), Some(43));
        assert_eq!(parse_percent("-5"), Some(0));
        assert_eq!(parse_percent("1e9"), Some(u16::MAX));
        assert_eq!(parse_percent("nan"), None);
        assert_eq!(parse_percent("inf"), None);
        assert_eq!(parse_percent(""), None);
        assert_eq!(parse_percent("abc"), None);
    }

    #[test]
    fn clamp_never_panics_on_inverted_bounds() {
        assert_eq!(clamp_range(50, 10, 100), 50);
        assert_eq!(clamp_range(5, 10, 100), 10);
        assert_eq!(clamp_range(500, 10, 100), 100);
        // min > max in config.toml used to panic inside u16::clamp
        assert_eq!(clamp_range(50, 100, 10), 50);
        assert_eq!(clamp_range(500, 100, 10), 100);
    }

    #[test]
    fn picture_modes_round_trip_and_unique() {
        for &(n, v) in PICTURE_MODES {
            assert_eq!(picture_mode_value(n), Some(v));
            assert_eq!(picture_mode_name(v), Some(n));
        }
        let mut vals: Vec<u16> = PICTURE_MODES.iter().map(|m| m.1).collect();
        vals.sort_unstable();
        vals.dedup();
        assert_eq!(vals.len(), PICTURE_MODES.len());
        assert_eq!(picture_mode_value("Nope"), None);
        assert_eq!(picture_mode_name(9999), None);
    }

    #[test]
    fn inputs_round_trip() {
        assert_eq!(input_value("HDMI 2"), Some(0x12));
        assert_eq!(input_name(0x0F), Some("DisplayPort"));
        assert_eq!(input_name(0x22), None);
        assert_eq!(input_value("USB-C"), None);
    }

    #[test]
    fn bridge_publish_does_not_block_when_broker_is_down() {
        // Nothing listens on this port; the event loop is never polled, so the
        // request queue (cap 64) fills up. Blocking publish() would hang here.
        let opts = MqttOptions::new("test-down", "127.0.0.1", 1);
        let (client, _conn) = Client::new(opts, 4);
        for _ in 0..50 {
            let _ = client.try_publish("t", QoS::AtMostOnce, false, b"x".as_slice());
        }
        assert!(client.try_publish("t", QoS::AtMostOnce, false, b"x".as_slice()).is_err());
    }
}
