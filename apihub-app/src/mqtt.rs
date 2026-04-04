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
        let _ = client.subscribe(&cmd_brightness, QoS::AtMostOnce);
        let _ = client.subscribe(&cmd_volume, QoS::AtMostOnce);
        let _ = client.subscribe(&cmd_picture, QoS::AtMostOnce);
        let _ = client.subscribe(&cmd_input, QoS::AtMostOnce);

        let tx = client.clone();

        // Spawn event loop thread (blocking iterator over Connection)
        thread::spawn(move || {
            for notification in connection.iter() {
                match notification {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        if let Ok(mut c) = conn.lock() { *c = true; }
                        let _ = client.subscribe(&cmd_brightness, QoS::AtMostOnce);
                        let _ = client.subscribe(&cmd_volume, QoS::AtMostOnce);
                        let _ = client.subscribe(&cmd_picture, QoS::AtMostOnce);
                        let _ = client.subscribe(&cmd_input, QoS::AtMostOnce);
                        eprintln!("[mqtt] connected to {}:{}", cfg.broker, cfg.port);
                    }
                    Ok(Event::Incoming(Incoming::Publish(msg))) => {
                        let payload_str = std::str::from_utf8(&msg.payload).unwrap_or("");
                        let bus = &cfg.bus;
                        let prefix = &cfg.topic_prefix;
                        let model = &cfg.monitor_model;

                        if msg.topic == cmd_brightness {
                            if let Ok(val) = payload_str.trim().parse::<f32>() {
                                let v = (val as u16).clamp(cfg.bri_min, cfg.bri_max);
                                let _ = ddc::ddc_write_vcp(bus, 0x10, v);
                                let _ = client.publish(format!("{}/number/{}/brightness/state", prefix, model), QoS::AtMostOnce, true, v.to_string().as_bytes());
                                if let Ok(mut l) = lc_thread.lock() { *l = Some(format!("brightness → {}", v)); }
                            }
                        } else if msg.topic == cmd_volume {
                            if let Ok(val) = payload_str.trim().parse::<f32>() {
                                let v = (val as u16).clamp(0, 100);
                                let _ = ddc::ddc_write_vcp(bus, 0x62, v);
                                let _ = client.publish(format!("{}/number/{}/volume/state", prefix, model), QoS::AtMostOnce, true, v.to_string().as_bytes());
                                if let Ok(mut l) = lc_thread.lock() { *l = Some(format!("volume → {}", v)); }
                            }
                        } else if msg.topic == cmd_picture {
                            let mode_val = match payload_str.trim() {
                                "Custom" => Some(45u16), "Reader" => Some(1), "Vivid" => Some(20),
                                "HDR Effect" => Some(22), "Cinema" => Some(46), "Color Weakness" => Some(6),
                                "FPS 1" => Some(30), "FPS 2" => Some(31), "RTS" => Some(39),
                                "sRGB" => Some(15), "DCI-P3" => Some(24), "EBU" => Some(25),
                                "Photo" => Some(48), "Calibration" => Some(49), _ => None,
                            };
                            if let Some(v) = mode_val {
                                let _ = ddc::ddc_write_vcp(bus, 0x15, v);
                                let _ = client.publish(format!("{}/select/{}/picture_mode/state", prefix, model), QoS::AtMostOnce, true, payload_str.trim().as_bytes());
                                if let Ok(mut l) = lc_thread.lock() { *l = Some(format!("mode → {}", payload_str.trim())); }
                            }
                        } else if msg.topic == cmd_input {
                            let input_val = match payload_str.trim() {
                                "DisplayPort" => Some(0x0Fu16), "HDMI 1" => Some(0x11), "HDMI 2" => Some(0x12), _ => None,
                            };
                            if let Some(v) = input_val {
                                let _ = ddc::ddc_write_vcp(bus, 0x60, v);
                                let _ = client.publish(format!("{}/select/{}/input_source/state", prefix, model), QoS::AtMostOnce, true, payload_str.trim().as_bytes());
                                if let Ok(mut l) = lc_thread.lock() { *l = Some(format!("input → {}", payload_str.trim())); }
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

            let pct = kb.battery.percentage_interpolated
                .or(kb.battery.percentage_fine)
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
            let _ = tx.publish(
                format!("{}/binary_sensor/apple_kb_{}/connected/config", prefix, mac),
                QoS::AtMostOnce, true, connected_config.as_bytes(),
            );
            let _ = tx.publish(
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
                    let _ = tx.publish(
                        format!("{}/sensor/apple_kb_{}/{}/config", prefix, mac, sid),
                        QoS::AtMostOnce, true, config.as_bytes(),
                    );
                    let _ = tx.publish(
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
                let _ = tx.publish(
                    format!("{}/sensor/{}/{}/config", prefix, model, sid),
                    QoS::AtMostOnce, true, config.as_bytes(),
                );
                let _ = tx.publish(
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
        let _ = tx.publish(
            format!("{}/number/{}/brightness/config", prefix, model),
            QoS::AtMostOnce, true, num_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("brightness") {
            let _ = tx.publish(
                format!("{}/number/{}/brightness/state", prefix, model),
                QoS::AtMostOnce, true, cur.to_string().as_bytes(),
            );
        }

        // ── Volume number entity ──────────────────────────────────
        let vol_config = format!(
            r#"{{"name":"LG Monitor Volume","unique_id":"{}_volume_ctrl","command_topic":"{}/number/{}/volume/set","state_topic":"{}/number/{}/volume/state","min":0,"max":100,"step":1,"unit_of_measurement":"%","icon":"mdi:volume-high","device":{}}}"#,
            model, prefix, model, prefix, model, device_mon
        );
        let _ = tx.publish(
            format!("{}/number/{}/volume/config", prefix, model),
            QoS::AtMostOnce, true, vol_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("volume") {
            let _ = tx.publish(
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
        let _ = tx.publish(
            format!("{}/select/{}/picture_mode/config", prefix, model),
            QoS::AtMostOnce, true, pm_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("picture_mode") {
            let name = match *cur {
                1 => "Reader", 6 => "Color Weakness", 15 => "sRGB",
                20 => "Vivid", 22 => "HDR Effect", 24 => "DCI-P3",
                25 => "EBU", 30 => "FPS 1", 31 => "FPS 2", 39 => "RTS",
                45 => "Custom", 46 => "Cinema", 48 => "Photo", 49 => "Calibration",
                _ => "Unknown",
            };
            let _ = tx.publish(
                format!("{}/select/{}/picture_mode/state", prefix, model),
                QoS::AtMostOnce, true, name.as_bytes(),
            );
        }

        // ── Input source select entity ─────────────────────────────
        let is_config = format!(
            r#"{{"name":"LG Input Source","unique_id":"{}_input_source","command_topic":"{}/select/{}/input_source/set","state_topic":"{}/select/{}/input_source/state","options":["DisplayPort","HDMI 1","HDMI 2"],"icon":"mdi:video-input-hdmi","device":{}}}"#,
            model, prefix, model, prefix, model, device_mon
        );
        let _ = tx.publish(
            format!("{}/select/{}/input_source/config", prefix, model),
            QoS::AtMostOnce, true, is_config.as_bytes(),
        );
        if let Some((cur, _)) = ddc_data.get("input_source") {
            let name = match *cur { 0x0F => "DisplayPort", 0x11 => "HDMI 1", 0x12 => "HDMI 2", _ => "Unknown" };
            let _ = tx.publish(
                format!("{}/select/{}/input_source/state", prefix, model),
                QoS::AtMostOnce, true, name.as_bytes(),
            );
        }

        if let Ok(mut lp) = self.last_publish.lock() {
            *lp = Some(std::time::Instant::now());
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.lock().map(|c| *c).unwrap_or(false)
    }
}
