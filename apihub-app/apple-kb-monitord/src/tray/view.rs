//! Pure part of the tray model: snapshot → tooltip text and the widget's menu.

use akm_core::Snapshot;
use std::fmt::Write as _;

fn pct_text(p: f64) -> String {
    tr!("{pct}%", pct = format!("{p:.0}"))
}

fn volts_text(v: f64) -> String {
    tr!("{volts} V", volts = format!("{v:.2}"))
}

/// "Apple Wireless Keyboard (A1314, aluminum, ISO)" → "Apple Wireless Keyboard".
pub fn short_model(model: &str) -> &str {
    model.split(" (").next().unwrap_or(model).trim()
}

/// RSSI is `BlueZ` MGMT `GET_CONN_INFO`.
pub fn rssi_text(rssi: Option<i32>) -> Option<String> {
    akm_core::signal::text(rssi)
}

/// Estimated charge by chemistry, next to the keyboard's own indication.
pub fn estimate_text(b: &akm_core::report::KbBattery) -> Option<String> {
    if b.new_batteries {
        return Some(tr!("New batteries: no estimate yet"));
    }
    let e = b.charge_estimate.as_ref()?;
    let chem = match e.chemistry {
        akm_core::chemistry::Chemistry::Alkaline => tr!("alkaline"),
        akm_core::chemistry::Chemistry::Nimh => "NiMH".to_string(),
        akm_core::chemistry::Chemistry::Lithium => tr!("lithium"),
        akm_core::chemistry::Chemistry::Unknown => return None,
    };
    Some(tr!(
        "Estimate ({chem}): \u{2248} {pct}% ({low} to {high}%)",
        chem = chem,
        pct = format!("{:.0}", e.pct),
        low = format!("{:.0}", e.low),
        high = format!("{:.0}", e.high)
    ))
}

pub fn age_text(last_update: u64, now: u64) -> String {
    if last_update == 0 {
        return tr!("Never updated");
    }
    let a = now.saturating_sub(last_update);
    let d = if a < 60 {
        format!("{a}\u{a0}s")
    } else if a < 3600 {
        format!("{}\u{a0}min", a / 60)
    } else if a < 86_400 {
        format!("{}\u{a0}h", a / 3600)
    } else {
        tr!("{n}\u{a0}d", n = a / 86_400)
    };
    tr!("Updated {d} ago", d = d)
}

/// Stable menu ids (the widget picks its entries by id).
pub mod id {
    pub const REFRESH: i32 = 12;
    pub const COPY: i32 = 13;
    pub const BLUETOOTH: i32 = 14;
    pub const RENAME: i32 = 15;
    /// Guided repair of the Bluetooth link.
    pub const REPAIR: i32 = 16;
    /// Fn mode of `hid_apple`: the two usual modes as radio items.
    pub const FN_MEDIA: i32 = 19;
    pub const FN_FKEYS: i32 = 22;
    /// Bluetooth actions: reconnect, disconnect, forget (confirmed).
    pub const RECONNECT: i32 = 23;
    pub const DISCONNECT: i32 = 24;
    pub const FORGET: i32 = 25;
    #[cfg(test)]
    pub const ALL: [i32; 10] = [
        REFRESH, COPY, BLUETOOTH, RENAME, REPAIR, RECONNECT, DISCONNECT, FORGET, FN_MEDIA, FN_FKEYS,
    ];
}

/// A menu entry property value (kept simple so it can be compared and tested).
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Str(String),
    Bool(bool),
    /// `toggle-state` (1 = checked, 0 = not).
    Int(i32),
}

/// One menu entry: id + properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i32,
    pub props: Vec<(&'static str, Prop)>,
}

impl Entry {
    pub fn get(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
    }
    pub fn visible(&self) -> bool {
        self.get("visible") != Some(&Prop::Bool(false))
    }
    pub fn enabled(&self) -> bool {
        self.get("enabled") != Some(&Prop::Bool(false))
    }
}

/// Qt text of a menu label: mnemonic `_x` → `&x`, `__` → `_`, `&` → `&&`.
pub fn qt_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len() + 1);
    let mut it = label.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '_' if it.peek() == Some(&'_') => {
                it.next();
                out.push('_');
            }
            '_' => out.push('&'),
            '&' => out.push_str("&&"),
            _ => out.push(c),
        }
    }
    out
}

fn hidden(id: i32) -> Entry {
    Entry {
        id,
        props: vec![
            ("label", Prop::Str(String::new())),
            ("enabled", Prop::Bool(false)),
            ("visible", Prop::Bool(false)),
        ],
    }
}

fn action(id: i32, label: &str, enabled: bool) -> Entry {
    let mut props = vec![("label", Prop::Str(label.into()))];
    if !enabled {
        props.push(("enabled", Prop::Bool(false)));
    }
    Entry { id, props }
}

/// `hid_apple.fnmode` values the menu offers: media keys first and F1-F12 first.
pub const FN_MEDIA_FIRST: u8 = 1;
pub const FN_FKEYS_FIRST: u8 = 2;

/// The radio item that matches `mode`, if any.
pub fn fn_mode_checked(mode: i32) -> Option<u8> {
    match mode {
        1 | 3 => Some(FN_MEDIA_FIRST),
        2 => Some(FN_FKEYS_FIRST),
        _ => None,
    }
}

fn fn_entries(mode: Option<i32>, has_keyboard: bool) -> [Entry; 2] {
    let Some(m) = mode.filter(|m| akm_core::hid_params::fn_mode_label(*m).is_some()) else {
        return [hidden(id::FN_MEDIA), hidden(id::FN_FKEYS)];
    };
    let radio = |id: i32, value: u8, label: &str| {
        let mut e = action(id, label, has_keyboard);
        e.props.push((
            "toggle-state",
            Prop::Int(i32::from(fn_mode_checked(m) == Some(value))),
        ));
        e
    };
    [
        radio(
            id::FN_MEDIA,
            FN_MEDIA_FIRST,
            &tr!("_Media keys first (Fn + F1 = F1)"),
        ),
        radio(
            id::FN_FKEYS,
            FN_FKEYS_FIRST,
            &tr!("F1–F12 _first (Fn + F1 = media)"),
        ),
    ]
}

/// The tray's action entries for one state (the widget's right-click menu).
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub menu: Vec<Entry>,
}

impl View {
    /// `fn_mode`: `hid_apple.fnmode` as read in sysfs (`None` = not loaded).
    pub fn build(snap: &Snapshot, fn_mode: Option<i32>) -> Self {
        let mut menu = vec![
            action(id::REFRESH, &tr!("_Refresh"), snap.connected),
            action(id::COPY, &tr!("_Copy information"), true),
            action(id::BLUETOOTH, &tr!("_Bluetooth settings…"), true),
            action(id::RENAME, &tr!("Re_name keyboard…"), snap.mac().is_some()),
            action(id::REPAIR, &tr!("Re_pair the link…"), true),
            action(
                id::RECONNECT,
                &tr!("Reconnec_t"),
                snap.mac().is_some() && !snap.connected,
            ),
            action(id::DISCONNECT, &tr!("_Disconnect"), snap.connected),
            action(
                id::FORGET,
                &tr!("For_get this keyboard…"),
                snap.mac().is_some(),
            ),
        ];
        menu.extend(fn_entries(fn_mode, snap.mac().is_some()));
        Self { menu }
    }

    /// The menu as JSON for the Plasma widget's right-click menu.
    pub fn menu_json(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .menu
            .iter()
            .map(|e| {
                let label = match e.get("label") {
                    Some(Prop::Str(l)) => qt_label(l),
                    _ => String::new(),
                };
                serde_json::json!({
                    "id": e.id,
                    "label": label,
                    "enabled": e.enabled(),
                    "visible": e.visible(),
                    "checked": match e.get("toggle-state") {
                        Some(Prop::Int(n)) => serde_json::Value::Bool(*n == 1),
                        _ => serde_json::Value::Null,
                    },
                })
            })
            .collect();
        serde_json::Value::Array(items).to_string()
    }

    #[cfg(test)]
    pub fn entry(&self, id: i32) -> Option<&Entry> {
        self.menu.iter().find(|e| e.id == id)
    }
}

/// What a state says in words: title and lines ("Copy information" starts with them).
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub title: String,
    pub lines: Vec<String>,
}

impl Summary {
    pub fn build(snap: &Snapshot, charging: bool) -> Self {
        // Several keyboards: the title is the weakest connected one's; everything else
        // describes the keyboard the daemon reads.
        let several = snap.devices.len() >= 2;
        let weakest = several
            .then(|| akm_core::roster::weakest(&snap.devices))
            .flatten();
        let other = weakest.filter(|w| !w.primary);
        let pct = snap.battery_pct();
        let model_full = snap
            .model()
            .map(str::to_string)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| tr!("Apple keyboard"));
        let model = short_model(&model_full).to_string();
        // The user's alias, else the name the keyboard registered under.
        let name = snap.display_name().map(str::to_string);
        let title_name = name.clone().unwrap_or_else(|| model.clone());

        let title = match (other, snap.connected, pct, snap.keyboard.is_some()) {
            (Some(w), ..) => match w.battery {
                Some(p) => format!("{} \u{2014} {}", w.name, pct_text(p)),
                None => w.name.clone(),
            },
            (None, true, Some(p), _) => format!("{title_name} \u{2014} {}", pct_text(p)),
            (None, true, None, _) => title_name,
            (None, false, _, true) => {
                format!("{title_name} \u{2014} {}", tr!("offline"))
            }
            (None, false, _, false) => tr!("Apple keyboard"),
        };
        let mut lines = Vec::new();
        if snap.keyboard.is_some() {
            lines.push(match &name {
                Some(n) => format!("{model_full} \u{b7} {n}"),
                None => model_full.clone(),
            });
        }
        if snap.connected {
            lines.extend(pct.map(|p| {
                let mut s = format!("{} {}", tr!("Keyboard indication:"), pct_text(p));
                if let Some(v) = snap.voltage().filter(|v| *v > 0.0) {
                    let _ = write!(s, " \u{b7} {}", volts_text(v));
                }
                if charging {
                    let _ = write!(s, " \u{b7} {}", tr!("charging"));
                }
                s
            }));
            let quiet = snap
                .keyboard
                .as_ref()
                .filter(|_| pct.is_some() && !charging);
            lines.extend(quiet.and_then(|k| estimate_text(&k.battery)));
            // Percentage as macOS shows it, labelled, next to the others.
            lines.extend(
                quiet
                    .and_then(|k| k.battery.apple_display_pct)
                    .map(|p| format!("{} {}", tr!("Apple display:"), pct_text(p))),
            );
            // No value: the reason in short, never silence.
            lines.extend(rssi_text(snap.rssi()).or_else(|| {
                let issue = snap.keyboard.as_ref()?.radio.rssi_error.as_ref()?;
                Some(akm_core::rssi::short_line(&issue.code))
            }));
            lines.extend(
                snap.remaining_text()
                    .map(|r| format!("{} {r}", tr!("Estimated runtime:"))),
            );
            lines.extend(
                snap.keyboard
                    .as_ref()
                    .and_then(|k| akm_core::firmware::summary(&k.firmware)),
            );
            if snap.caps_lock {
                lines.push(tr!("Caps Lock on"));
            }
        } else {
            if let Some(p) = pct {
                lines.push(format!("{} {}", tr!("Last value:"), pct_text(p)));
            }
            if snap.keyboard.is_none() {
                lines.push(
                    snap.kb_error
                        .clone()
                        .filter(|e| !e.is_empty())
                        .unwrap_or_else(|| tr!("No keyboard found")),
                );
            }
        }
        // "Batteries changed too often": shown connected or not.
        lines.extend(snap.battery_advice.as_ref().map(akm_core::advice::line));
        if several {
            lines.extend(keyboard_lines(&snap.devices, weakest));
        }
        Self { title, lines }
    }
}

/// One line per keyboard when there are several: name, level, link, and which is the weakest.
fn keyboard_lines(
    devices: &[akm_core::roster::DeviceSummary],
    weakest: Option<&akm_core::roster::DeviceSummary>,
) -> Vec<String> {
    // Never "the weakest" next to a keyboard whose level is unknown.
    let all_known = devices
        .iter()
        .filter(|o| o.connected)
        .all(|o| o.battery.is_some_and(f64::is_finite));
    devices
        .iter()
        .map(|d| {
            let level = d.battery.map_or_else(|| tr!("level unknown"), pct_text);
            let link = if d.connected {
                tr!("connected")
            } else {
                tr!("offline")
            };
            if all_known && weakest.is_some_and(|w| w.mac == d.mac) {
                tr!(
                    "{name} \u{2014} {level} · {link} · weakest",
                    name = d.name,
                    level = level,
                    link = link
                )
            } else {
                format!("{} \u{2014} {level} · {link}", d.name)
            }
        })
        .collect()
}

/// Plain-text block for the clipboard ("Copy the information").
pub fn clipboard_text(snap: &Snapshot, charging: bool, now: u64) -> String {
    let s = Summary::build(snap, charging);
    let mut out = vec![s.title];
    out.extend(s.lines);
    if let Some(k) = &snap.keyboard {
        if let Some(m) = k.device.mac.as_deref().filter(|m| !m.is_empty()) {
            out.push(format!("{} {}", tr!("MAC:"), akm_core::power::mask_mac(m)));
        }
        let fw = akm_core::firmware::summary(&k.firmware);
        if let Some(fw) = fw {
            out.push(fw);
        }
    }
    if snap.keyboard.is_some() {
        out.push(age_text(snap.last_update, now));
    }
    if let Some(e) = snap.last_error.as_deref().filter(|e| !e.is_empty()) {
        out.push(format!("{} {e}", tr!("Last error:")));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::KbReport;

    #[test]
    fn qt_label_turns_mnemonics_into_qt_accelerators() {
        assert_eq!(qt_label("_Rename keyboard…"), "&Rename keyboard…");
        assert_eq!(qt_label("Re_nommer"), "Re&nommer");
        assert_eq!(qt_label("my__kbd"), "my_kbd");
        assert_eq!(qt_label("Q&A"), "Q&&A");
        // CJK catalogs keep the accelerator after the text: no stray "(R)".
        assert_eq!(qt_label("更新(_R)"), "更新(&R)");
        assert_eq!(qt_label(""), "");
    }

    #[test]
    fn age_text_keeps_the_unit_with_its_number() {
        assert_eq!(age_text(0, 5), "Never updated");
        assert_eq!(age_text(1_000, 1_030), "Updated 30\u{a0}s ago");
        assert_eq!(age_text(1, 1 + 3 * 86_400), "Updated 3\u{a0}d ago");
    }

    #[test]
    fn menu_json_lists_the_action_entries_with_qt_labels() {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(80.0);
        let s = Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        };
        let v = View::build(&s, Some(2));
        let j: serde_json::Value = serde_json::from_str(&v.menu_json()).unwrap();
        let items = j.as_array().unwrap();
        assert_eq!(items.len(), 10, "{j}");
        assert!(items.iter().all(|i| super::super::action_for(
            i32::try_from(i["id"].as_i64().unwrap()).unwrap()
        )
        .is_some()));
        let refresh = items.iter().find(|i| i["id"] == id::REFRESH).unwrap();
        assert_eq!(refresh["label"], qt_label(&tr!("_Refresh")));
        assert_eq!(refresh["enabled"], true);
        assert_eq!(refresh["checked"], serde_json::Value::Null);
        let fkeys = items.iter().find(|i| i["id"] == id::FN_FKEYS).unwrap();
        assert!(fkeys["checked"].is_boolean(), "{fkeys}");
    }

    #[test]
    fn two_keyboards_the_summary_shows_the_weakest_and_lists_both() {
        use akm_core::roster::DeviceSummary;
        let dev = |mac: &str, name: &str, connected: bool, battery: Option<f64>, primary: bool| {
            DeviceSummary {
                mac: mac.into(),
                name: name.into(),
                connected,
                battery,
                primary,
            }
        };
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(80.0);
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        k.device.alias = Some("Desk".into());
        let mut s = Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        };
        s.devices = vec![dev("AA:BB:CC:DD:EE:F1", "Desk", true, Some(80.0), true)];
        let one = Summary::build(&s, false);
        assert!(
            !one.lines.iter().any(|l| l.contains("connected")),
            "{one:?}"
        );
        s.devices
            .push(dev("AA:BB:CC:DD:EE:F2", "Salon", true, Some(8.0), false));
        let v = Summary::build(&s, false);
        assert_eq!(v.title, "Salon \u{2014} 8%");
        let tail = |v: &Summary| v.lines[v.lines.len() - 2..].to_vec();
        assert_eq!(
            tail(&v),
            [
                "Desk \u{2014} 80% · connected",
                "Salon \u{2014} 8% · connected · weakest"
            ]
        );
        assert!(v.lines.iter().any(|l| l.contains("80")));
        let mut u = s.clone();
        u.devices[1].battery = None;
        assert!(!Summary::build(&u, false)
            .lines
            .join("\n")
            .contains("weakest"));
        s.devices[1].connected = false;
        let v2 = Summary::build(&s, false);
        assert_eq!(
            tail(&v2),
            [
                "Desk \u{2014} 80% · connected · weakest",
                "Salon \u{2014} 8% · offline"
            ]
        );
        assert!(v2.title.starts_with("Desk"));
        s.connected = false;
        s.devices[0].connected = false;
        s.devices[1].connected = true;
        assert_eq!(
            tail(&Summary::build(&s, false)),
            [
                "Desk \u{2014} 80% · offline",
                "Salon \u{2014} 8% · connected · weakest"
            ]
        );
        // Unknown level: said, never invented.
        s.devices[1].battery = None;
        assert_eq!(
            tail(&Summary::build(&s, false))[1],
            "Salon \u{2014} level unknown · connected"
        );
    }

    #[test]
    fn bluetooth_actions_follow_the_link_state() {
        let enabled = |v: &View, i: i32| v.entry(i).unwrap().enabled();
        let none = View::build(&Snapshot::default(), None);
        assert!(!enabled(&none, id::RECONNECT) && !enabled(&none, id::DISCONNECT));
        assert!(!enabled(&none, id::FORGET));
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(90.0);
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        let mut s = Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        };
        let on = View::build(&s, None);
        assert!(!enabled(&on, id::RECONNECT), "already connected");
        assert!(enabled(&on, id::DISCONNECT) && enabled(&on, id::FORGET));
        s.connected = false;
        let off = View::build(&s, None);
        assert!(enabled(&off, id::RECONNECT) && !enabled(&off, id::DISCONNECT));
        assert!(enabled(&off, id::FORGET));
        let label = |v: &View, i: i32| match v.entry(i).unwrap().get("label") {
            Some(Prop::Str(l)) => l.clone(),
            _ => String::new(),
        };
        assert_eq!(label(&off, id::FORGET), "For_get this keyboard…");
        assert_eq!(label(&off, id::RECONNECT), "Reconnec_t");
        assert_eq!(label(&off, id::DISCONNECT), "_Disconnect");
    }

    #[test]
    fn caps_lock_line_shows_only_while_connected() {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(90.0);
        let mut s = Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        };
        let caps = |s: &Snapshot| {
            Summary::build(s, false)
                .lines
                .contains(&"Caps Lock on".into())
        };
        assert!(!caps(&s));
        s.caps_lock = true;
        assert!(caps(&s));
        s.connected = false;
        assert!(!caps(&s));
    }

    #[test]
    fn battery_advice_line_appears_only_when_due() {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(90.0);
        let mut s = Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        };
        let line = "Batteries replaced too often: 12 d then 18 d (under 30 d)".to_string();
        assert!(!Summary::build(&s, false).lines.contains(&line));
        s.battery_advice = Some(akm_core::advice::ShortLife {
            days: [12.0, 18.0],
            since: 1,
        });
        assert!(Summary::build(&s, false).lines.contains(&line));
        s.connected = false;
        assert!(Summary::build(&s, false).lines.contains(&line));
    }

    #[test]
    fn fn_mode_radios_check_the_matching_item() {
        let radio = |v: &View, id: i32| v.entry(id).and_then(|e| e.get("toggle-state").cloned());
        let s = snap(Some(80.0), true, None);
        let v = View::build(&s, Some(2));
        assert_eq!(radio(&v, id::FN_FKEYS), Some(Prop::Int(1)));
        assert_eq!(radio(&v, id::FN_MEDIA), Some(Prop::Int(0)));
        for id in [id::FN_MEDIA, id::FN_FKEYS] {
            let e = v.entry(id).unwrap();
            assert_eq!(e.get("enabled"), None, "enabled with a keyboard");
            assert_eq!(e.get("toggle-type"), None, "only what menu_json reads");
        }
        let v = View::build(&s, Some(3));
        assert_eq!(radio(&v, id::FN_MEDIA), Some(Prop::Int(1)));
        let v = View::build(&s, Some(0));
        assert_eq!(
            (radio(&v, id::FN_MEDIA), radio(&v, id::FN_FKEYS)),
            (Some(Prop::Int(0)), Some(Prop::Int(0)))
        );
        for m in [None, Some(9)] {
            let v = View::build(&s, m);
            for id in [id::FN_MEDIA, id::FN_FKEYS] {
                assert!(!v.entry(id).unwrap().visible(), "{m:?} {id}");
            }
        }
        let v = View::build(&Snapshot::default(), Some(1));
        assert!(!v.entry(id::FN_MEDIA).unwrap().enabled());
        assert!(v.entry(id::FN_MEDIA).unwrap().visible());
    }

    fn snap(pct: Option<f64>, connected: bool, rssi: Option<i32>) -> Snapshot {
        let mut k = KbReport::default();
        k.battery.percentage_fine = pct;
        k.battery.voltage = Some(2.903);
        k.radio.rssi_dbm = rssi;
        k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
        k.device.name = Some("Keyboard_test".into());
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        Snapshot {
            connected,
            keyboard: Some(k),
            last_update: 1_000,
            ..Default::default()
        }
    }

    #[test]
    fn rssi_unknown_is_absent_never_127() {
        assert_eq!(rssi_text(None), None);
        assert_eq!(rssi_text(Some(127)), None);
        assert_eq!(rssi_text(Some(0)).unwrap(), "Signal: excellent (0)");
        assert_eq!(rssi_text(Some(-48)).unwrap(), "Signal: weak (\u{2212}48)");
        assert_eq!(rssi_text(Some(-4)).unwrap(), "Signal: good (\u{2212}4)");
        assert_eq!(rssi_text(Some(2)).unwrap(), "Signal: excellent (+2)");
        for r in [0, -4, 2, -48] {
            assert!(!rssi_text(Some(r)).unwrap().contains("dBm"));
        }

        let body = Summary::build(&snap(Some(99.0), true, None), false)
            .lines
            .join("\n");
        assert!(!body.contains("127") && !body.contains("Signal"), "{body}");
        let body = Summary::build(&snap(Some(99.0), true, Some(127)), false)
            .lines
            .join("\n");
        assert!(!body.contains("127"));
        let mut s = snap(Some(99.0), true, None);
        s.keyboard.as_mut().unwrap().radio.rssi_error = Some(akm_core::rssi::classify(
            &akm_core::rssi::RssiError::HelperMissing(String::new()),
        ));
        let body = Summary::build(&s, false).lines.join("\n");
        assert!(
            body.contains("not measured (rssi-helper not installed)"),
            "{body}"
        );
    }

    #[test]
    fn menu_is_complete_and_stable() {
        let v = View::build(&snap(Some(99.0), true, Some(0)), Some(1));
        let ids: Vec<i32> = v.menu.iter().map(|e| e.id).collect();
        assert_eq!(ids, id::ALL.to_vec());
        let v2 = View::build(&snap(Some(50.0), false, None), None);
        let ids2: Vec<i32> = v2.menu.iter().map(|e| e.id).collect();
        assert_eq!(ids, ids2, "ids never change, only visibility");
        assert!(!v2.entry(id::REFRESH).unwrap().enabled());
    }

    #[test]
    fn clipboard_block() {
        let t = clipboard_text(&snap(Some(99.0), true, Some(-3)), false, 1_030);
        assert!(t.starts_with("Keyboard_test \u{2014} 99%"), "{t}");
        assert!(t.contains("Keyboard indication: 99% \u{b7} 2.90"), "{t}");
        // the copied text may leave the machine: the address is masked as in the KCM.
        assert!(t.contains("MAC: AA:BB:XX:XX:XX:F1"), "{t}");
        assert!(!t.contains("CC:DD:EE"), "{t}");
        assert!(t.contains("Updated 30\u{a0}s ago"), "{t}");
        let t = clipboard_text(&snap(Some(99.0), true, None), false, 1_030);
        assert!(t.contains("MAC: AA:BB:XX:XX:XX:F1"), "{t}");
        assert!(!t.contains("127"));
    }

    #[test]
    fn alias_wins_everywhere_and_rename_is_offered() {
        let mut s = snap(Some(99.0), true, Some(-3));
        s.keyboard.as_mut().unwrap().device.alias = Some("Alice's keyboard #1".into());
        let sum = Summary::build(&s, false);
        assert_eq!(sum.title, "Alice's keyboard #1 \u{2014} 99%");
        assert_eq!(
            sum.lines[0],
            "Apple Wireless Keyboard (A1314, aluminum, ISO) \u{b7} Alice's keyboard #1"
        );
        let v = View::build(&s, None);
        let r = v.entry(id::RENAME).unwrap();
        assert_eq!(
            r.get("label"),
            Some(&Prop::Str("Re_name keyboard\u{2026}".into()))
        );
        assert!(r.enabled(), "enabled while a keyboard is known");
        let none = View::build(&Snapshot::default(), None);
        assert!(!none.entry(id::RENAME).unwrap().enabled());
    }
}
