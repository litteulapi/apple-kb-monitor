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
        let cmd_topic = format!(
            "{}/number/{}/brightness/set",
            cfg.topic_prefix, cfg.monitor_model
        );
        let _ = client.subscribe(&cmd_topic, QoS::AtMostOnce);

        let tx = client.clone();

        // Spawn event loop thread (blocking iterator over Connection)
        thread::spawn(move || {
            for notification in connection.iter() {
                match notification {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        if let Ok(mut c) = conn.lock() { *c = true; }
                        let _ = client.subscribe(&cmd_topic, QoS::AtMostOnce);
                        eprintln!("[mqtt] connected to {}:{}", cfg.broker, cfg.port);
                    }
                    Ok(Event::Incoming(Incoming::Publish(msg))) => {
                        if msg.topic == cmd_topic {
                            if let Ok(payload) = std::str::from_utf8(&msg.payload) {
                                if let Ok(val) = payload.trim().parse::<f32>() {
                                    let clamped = (val as u16).clamp(cfg.bri_min, cfg.bri_max);
                                    let bus = &cfg.bus;
                                    if let Err(e) = ddc::ddc_write_vcp(bus, 0x10, clamped) {
                                        eprintln!("[mqtt] DDC write error: {}", e);
                                    } else {
                                        eprintln!("[mqtt] brightness → {}", clamped);
                                        // Publish state back
                                        let state_topic = format!(
                                            "{}/number/{}/brightness/state",
                                            cfg.topic_prefix, cfg.monitor_model
                                        );
                                        let _ = client.publish(
                                            &state_topic, QoS::AtMostOnce, true,
                                            clamped.to_string().as_bytes(),
                                        );
                                    }
                                    if let Ok(mut l) = lc_thread.lock() {
                                        *l = Some(format!("brightness → {}", clamped));
                                    }
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
            connected: Arc::new(Mutex::new(false)),
            last_publish: lp,
            last_cmd: lc,
            tx: Some(tx),
        }
    }

    /// Publish all keyboard + monitor data to HA auto-discovery.
    pub fn publish_telemetry(
        &self,
        kb: &Option<crate::KbReport>,
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
            ];

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

        if let Ok(mut lp) = self.last_publish.lock() {
            *lp = Some(std::time::Instant::now());
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.lock().map(|c| *c).unwrap_or(false)
    }
}
