//! Pure part of the tray: snapshot → icon name, status, tooltip, menu.
//! No I/O here, everything is unit-tested (docs/REVUE-UI-TRAY.md §5).

use akm_core::Snapshot;

/// Below or at this level the icon turns to "caution" and the item asks for
/// attention.
pub const CRITICAL_PCT: f64 = 10.0;
/// Below or at this level menu entries get the `warning` disposition.
pub const LOW_PCT: f64 = 20.0;
/// Hysteresis around bucket edges (percentage points).
pub const HYSTERESIS: f64 = 2.0;

// ── Language ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    /// From `LC_ALL` > `LC_MESSAGES` > `LANG` (first non-empty), French if it
    /// starts with `fr`, English otherwise.
    pub fn from_env() -> Self {
        let v = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        Self::from_locale(&v)
    }
    pub fn from_locale(v: &str) -> Self {
        if v.to_ascii_lowercase().starts_with("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }
    pub(crate) fn t(self, fr: &'static str, en: &'static str) -> &'static str {
        match self {
            Lang::Fr => fr,
            Lang::En => en,
        }
    }
    fn pct(self, p: f64) -> String {
        match self {
            Lang::Fr => format!("{:.0}\u{a0}%", p),
            Lang::En => format!("{:.0}%", p),
        }
    }
    fn volts(self, v: f64) -> String {
        let s = format!("{v:.2}");
        match self {
            Lang::Fr => format!("{}\u{a0}V", s.replace('.', ",")),
            Lang::En => format!("{s} V"),
        }
    }
}

// ── Icon ────────────────────────────────────────────────────────────────────

/// Which naming scheme the icon names follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    /// Our set `apihub-kb-*-symbolic` (icons/hicolor/scalable/status).
    Ours,
    /// Breeze: `battery-050-symbolic`.
    Breeze,
    /// Adwaita / freedesktop: `battery-level-50-symbolic`.
    Adwaita,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// Bucket 0..=100 by steps of 10.
    Level { bucket: u8, charging: bool },
    /// ≤ [`CRITICAL_PCT`] and not charging.
    Caution,
    /// Keyboard known but offline.
    Disconnected,
    /// No keyboard / no battery value.
    Missing,
}

impl IconState {
    pub fn name(self, set: IconSet) -> String {
        match (set, self) {
            (IconSet::Ours, IconState::Level { bucket, charging }) => format!(
                "apihub-kb-battery-{bucket:03}{}-symbolic",
                if charging { "-charging" } else { "" }
            ),
            (IconSet::Ours, IconState::Caution) => "apihub-kb-battery-caution-symbolic".into(),
            (IconSet::Ours, IconState::Disconnected) => "apihub-kb-disconnected-symbolic".into(),
            (IconSet::Ours, IconState::Missing) => "apihub-kb-missing-symbolic".into(),
            (IconSet::Breeze, IconState::Level { bucket, charging }) => format!(
                "battery-{bucket:03}{}-symbolic",
                if charging { "-charging" } else { "" }
            ),
            (IconSet::Adwaita, IconState::Level { bucket, charging }) => format!(
                "battery-level-{bucket}{}-symbolic",
                if charging { "-charging" } else { "" }
            ),
            (_, IconState::Caution) => "battery-caution-symbolic".into(),
            (IconSet::Adwaita, IconState::Disconnected) => "battery-missing-symbolic".into(),
            (_, IconState::Disconnected | IconState::Missing) => "input-keyboard-symbolic".into(),
        }
    }
}

/// Bucket choice with hysteresis: the previous bucket `prev` is kept while
/// `pct` stays within `[prev − 2, prev + 10 + 2)`, so 49-51 % does not flap.
/// Plain rounding down otherwise (never "100" below 100 %).
pub fn bucket(pct: f64, prev: Option<u8>) -> u8 {
    let pct = pct.clamp(0.0, 100.0);
    let plain = ((pct / 10.0).floor() * 10.0) as u8;
    match prev {
        Some(b) if b <= 100 => {
            let lo = f64::from(b) - HYSTERESIS;
            let hi = (f64::from(b) + 10.0 + HYSTERESIS).min(100.0);
            if pct >= lo && (pct < hi || b == 100) {
                b
            } else {
                plain
            }
        }
        _ => plain,
    }
}

pub fn icon_state(snap: &Snapshot, charging: bool, prev_bucket: Option<u8>) -> IconState {
    if !snap.connected {
        return if snap.keyboard.is_some() {
            IconState::Disconnected
        } else {
            IconState::Missing
        };
    }
    match snap.battery_pct() {
        None => IconState::Missing,
        Some(p) if p <= CRITICAL_PCT && !charging => IconState::Caution,
        Some(p) => IconState::Level {
            bucket: bucket(p, prev_bucket),
            charging,
        },
    }
}

// ── Texts ───────────────────────────────────────────────────────────────────

/// "Apple Wireless Keyboard (A1314, aluminum, ISO)" → "Apple Wireless Keyboard".
pub fn short_model(model: &str) -> &str {
    model.split(" (").next().unwrap_or(model).trim()
}

/// RSSI is BlueZ MGMT `GET_CONN_INFO`: on BR/EDR it is a gap in dB to the
/// controller's ideal reception range (0 = ideal), **not** dBm (#174). Words
/// first, raw value in parentheses without unit. `None` = unknown: the line is
/// omitted, never "127".
pub fn rssi_text(lang: Lang, rssi: Option<i32>) -> Option<String> {
    akm_core::signal::text(rssi, lang == Lang::Fr)
        // French typography: no-break space before the colon.
        .map(|t| t.replacen(" :", "\u{a0}:", 1))
}

/// Estimated charge by chemistry, next to the keyboard's own indication (#178).
pub fn estimate_text(lang: Lang, b: &akm_core::report::KbBattery) -> Option<String> {
    if b.new_batteries {
        return Some(
            lang.t(
                "Piles neuves : pas d\u{2019}estimation",
                "New batteries: no estimate yet",
            )
            .into(),
        );
    }
    let e = b.charge_estimate.as_ref()?;
    let chem = match (lang, e.chemistry) {
        (Lang::Fr, akm_core::chemistry::Chemistry::Alkaline) => "alcaline",
        (_, akm_core::chemistry::Chemistry::Alkaline) => "alkaline",
        (_, akm_core::chemistry::Chemistry::Nimh) => "NiMH",
        (_, akm_core::chemistry::Chemistry::Lithium) => "lithium",
        (_, akm_core::chemistry::Chemistry::Unknown) => return None,
    };
    Some(match lang {
        Lang::Fr => format!(
            "Estimation ({chem}) :\u{a0}\u{2248}\u{a0}{:.0}\u{a0}% ({:.0} \u{e0} {:.0}\u{a0}%)",
            e.pct, e.low, e.high
        ),
        Lang::En => format!(
            "Estimate ({chem}): \u{2248} {:.0}% ({:.0} to {:.0}%)",
            e.pct, e.low, e.high
        ),
    })
}

pub fn age_text(lang: Lang, last_update: u64, now: u64) -> String {
    if last_update == 0 {
        return lang.t("Jamais mis à jour", "Never updated").into();
    }
    let a = now.saturating_sub(last_update);
    let d = if a < 60 {
        format!("{a}\u{a0}s")
    } else if a < 3600 {
        format!("{}\u{a0}min", a / 60)
    } else if a < 86_400 {
        format!("{}\u{a0}h", a / 3600)
    } else {
        format!("{}\u{a0}{}", a / 86_400, lang.t("j", "d"))
    };
    match lang {
        Lang::Fr => format!("Mis à jour il y a {d}"),
        Lang::En => format!("Updated {d} ago"),
    }
}

/// SNI `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    NeedsAttention,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "Active",
            Status::NeedsAttention => "NeedsAttention",
        }
    }
}

// ── Menu model ──────────────────────────────────────────────────────────────

/// Stable dbusmenu ids.
pub mod id {
    pub const ROOT: i32 = 0;
    pub const HEADER: i32 = 1;
    pub const BATTERY: i32 = 2;
    pub const CONNECTION: i32 = 3;
    pub const SIGNAL: i32 = 4;
    pub const AUTONOMY: i32 = 5;
    pub const CAPS: i32 = 6;
    /// Charge estimated by chemistry (#178).
    pub const ESTIMATE: i32 = 7;
    pub const SEP1: i32 = 10;
    pub const OPEN: i32 = 11;
    pub const REFRESH: i32 = 12;
    pub const COPY: i32 = 13;
    pub const BLUETOOTH: i32 = 14;
    pub const RENAME: i32 = 15;
    /// Guided repair of the Bluetooth link (#147).
    pub const REPAIR: i32 = 16;
    pub const SEP2: i32 = 20;
    pub const QUIT: i32 = 21;
    #[cfg(test)]
    pub const ALL: [i32; 16] = [
        HEADER, BATTERY, ESTIMATE, CONNECTION, SIGNAL, AUTONOMY, CAPS, SEP1, OPEN, REFRESH, COPY, BLUETOOTH,
        RENAME, REPAIR, SEP2, QUIT,
    ];
}

/// A dbusmenu property value (kept simple so it can be compared and tested).
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Str(String),
    Bool(bool),
}

/// One menu entry: id + dbusmenu properties (only non-default ones, as the
/// spec requires: `enabled`/`visible` default to true, `type` to standard).
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
}

/// dbusmenu labels use `_` as mnemonic marker: escape data.
fn esc(s: &str) -> String {
    s.replace('_', "__")
}

fn info(id: i32, label: String, a11y: Option<String>) -> Entry {
    // Information rows stay ENABLED (readable, not greyed out): a disabled
    // dbusmenu entry looks "locked" in Plasma. Clicking one does nothing.
    let mut props = vec![("label", Prop::Str(esc(&label)))];
    if let Some(a) = a11y {
        props.push(("accessible-desc", Prop::Str(a)));
    }
    Entry { id, props }
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

fn action(id: i32, label: &str, icon: &str, enabled: bool) -> Entry {
    let mut props = vec![
        ("label", Prop::Str(label.into())),
        ("icon-name", Prop::Str(icon.into())),
    ];
    if !enabled {
        props.push(("enabled", Prop::Bool(false)));
    }
    Entry { id, props }
}

fn sep(id: i32) -> Entry {
    Entry {
        id,
        props: vec![("type", Prop::Str("separator".into()))],
    }
}

/// Everything the tray shows for one state (except the data age, which is
/// computed when the host reads the tooltip, so that time passing does not
/// count as a change).
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub icon: IconState,
    pub status: Status,
    pub tooltip_title: String,
    /// Tooltip lines without the "updated … ago" line.
    pub tooltip_lines: Vec<String>,
    pub last_update: u64,
    pub menu: Vec<Entry>,
}

impl View {
    pub fn build(snap: &Snapshot, charging: bool, prev_bucket: Option<u8>, lang: Lang) -> Self {
        let icon = icon_state(snap, charging, prev_bucket);
        let pct = snap.battery_pct();
        let model_full = snap
            .model()
            .map(str::to_string)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| lang.t("Clavier Apple", "Apple keyboard").into());
        let model = short_model(&model_full).to_string();
        // The user's alias (#141), else the name the keyboard registered under.
        let name = snap.display_name().map(str::to_string);
        let title_name = name.clone().unwrap_or_else(|| model.clone());

        let status = match (snap.connected, pct) {
            (true, Some(p)) if p <= CRITICAL_PCT && !charging => Status::NeedsAttention,
            _ => Status::Active,
        };

        let battery_line = pct.map(|p| {
            let mut s = format!(
                "{} {}",
                lang.t("Indication du clavier :", "Keyboard indication:"),
                lang.pct(p)
            );
            if let Some(v) = snap.voltage().filter(|v| *v > 0.0) {
                s += &format!(" \u{b7} {}", lang.volts(v));
            }
            if charging {
                s += &format!(" \u{b7} {}", lang.t("en charge", "charging"));
            }
            s
        });
        let battery_a11y = pct.map(|p| {
            let mut s = match lang {
                Lang::Fr => format!("Indication du clavier {:.0} pour cent", p),
                Lang::En => format!("Keyboard indication {:.0} percent", p),
            };
            if let Some(v) = snap.voltage().filter(|v| *v > 0.0) {
                s += &match lang {
                    Lang::Fr => format!(", {} volts", format!("{v:.2}").replace('.', ",")),
                    Lang::En => format!(", {v:.2} volts"),
                };
            }
            s
        });
        let rssi = if snap.connected {
            rssi_text(lang, snap.rssi())
        } else {
            None
        };
        let estimate = snap
            .keyboard
            .as_ref()
            .filter(|_| pct.is_some() && !charging)
            .and_then(|k| estimate_text(lang, &k.battery));
        // Percentage as macOS shows it (#213), labelled, next to the others.
        let apple = snap
            .keyboard
            .as_ref()
            .filter(|_| pct.is_some() && !charging)
            .and_then(|k| k.battery.apple_display_pct)
            .map(|p| format!("{} {}", lang.t("Affichage Apple :", "Apple display:"), lang.pct(p)));
        // Firmware check (#227), one line.
        let firmware = snap.keyboard.as_ref().and_then(|k| match lang {
            Lang::Fr => akm_core::firmware::summary_fr(&k.firmware),
            Lang::En => akm_core::firmware::summary_en(&k.firmware),
        });
        let autonomy = snap
            .remaining_display
            .as_ref()
            .filter(|r| !r.is_empty())
            .map(|r| {
                format!(
                    "{} {r}",
                    lang.t("Autonomie estimée :", "Estimated runtime:")
                )
            });
        let conn_line = if snap.connected {
            lang.t("Connecté", "Connected").to_string()
        } else if snap.keyboard.is_some() {
            lang.t("Hors ligne", "Offline").to_string()
        } else {
            snap.kb_error
                .clone()
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| lang.t("Aucun clavier détecté", "No keyboard found").into())
        };

        // Tooltip
        let tooltip_title = match (snap.connected, pct, snap.keyboard.is_some()) {
            (true, Some(p), _) => format!("{title_name} \u{2014} {}", lang.pct(p)),
            (true, None, _) => title_name.clone(),
            (false, _, true) => {
                format!("{title_name} \u{2014} {}", lang.t("hors ligne", "offline"))
            }
            (false, _, false) => lang.t("Clavier Apple", "Apple keyboard").into(),
        };
        let mut tooltip_lines = Vec::new();
        if snap.keyboard.is_some() {
            tooltip_lines.push(match &name {
                Some(n) => format!("{model_full} \u{b7} {n}"),
                None => model_full.clone(),
            });
        }
        if snap.connected {
            tooltip_lines.extend(battery_line.clone());
            tooltip_lines.extend(estimate.clone());
            tooltip_lines.extend(apple.clone());
            tooltip_lines.extend(rssi.clone());
            tooltip_lines.extend(autonomy.clone());
            tooltip_lines.extend(firmware.clone());
            if snap.caps_lock {
                tooltip_lines.push(lang.t("Verr. Maj active", "Caps Lock on").into());
            }
        } else {
            if let Some(p) = pct {
                tooltip_lines.push(format!(
                    "{} {}",
                    lang.t("Dernière valeur :", "Last value:"),
                    lang.pct(p)
                ));
            }
            if snap.keyboard.is_none() {
                tooltip_lines.push(conn_line.clone());
            }
        }

        // Menu
        let disposition = match pct {
            Some(p) if snap.connected && p <= CRITICAL_PCT && !charging => Some("alert"),
            Some(p) if snap.connected && p <= LOW_PCT && !charging => Some("warning"),
            _ => None,
        };
        let mut header = info(
            id::HEADER,
            name.clone().unwrap_or_else(|| model_full.clone()),
            None,
        );
        header
            .props
            .push(("icon-name", Prop::Str("input-keyboard-symbolic".into())));
        let battery = match (&battery_line, snap.connected) {
            (Some(l), true) => {
                let mut e = info(id::BATTERY, l.clone(), battery_a11y);
                if let Some(d) = disposition {
                    e.props.push(("disposition", Prop::Str(d.into())));
                }
                e
            }
            _ => hidden(id::BATTERY),
        };
        let menu = vec![
            header,
            battery,
            if snap.connected {
                estimate.map_or_else(|| hidden(id::ESTIMATE), |l| info(id::ESTIMATE, l, None))
            } else {
                hidden(id::ESTIMATE)
            },
            info(id::CONNECTION, conn_line, None),
            rssi.map_or_else(|| hidden(id::SIGNAL), |l| info(id::SIGNAL, l, None)),
            autonomy.map_or_else(|| hidden(id::AUTONOMY), |l| info(id::AUTONOMY, l, None)),
            if snap.connected && snap.caps_lock {
                info(
                    id::CAPS,
                    lang.t("Verr. Maj active", "Caps Lock on").into(),
                    None,
                )
            } else {
                hidden(id::CAPS)
            },
            sep(id::SEP1),
            action(
                id::OPEN,
                lang.t("_Ouvrir la fenêtre…", "_Open window…"),
                "window-new-symbolic",
                true,
            ),
            action(
                id::REFRESH,
                lang.t("_Rafraîchir", "_Refresh"),
                "view-refresh-symbolic",
                snap.connected,
            ),
            action(
                id::COPY,
                lang.t("_Copier les informations", "_Copy information"),
                "edit-copy-symbolic",
                true,
            ),
            action(
                id::BLUETOOTH,
                lang.t("Paramètres _Bluetooth…", "_Bluetooth settings…"),
                "preferences-system-bluetooth-symbolic",
                true,
            ),
            action(
                id::RENAME,
                lang.t("Re_nommer le clavier…", "Re_name keyboard…"),
                "edit-rename-symbolic",
                snap.mac().is_some(),
            ),
            action(
                id::REPAIR,
                lang.t("Ré_parer la liaison…", "Re_pair the link…"),
                "network-wireless-disconnected-symbolic",
                true,
            ),
            sep(id::SEP2),
            action(
                id::QUIT,
                lang.t("_Quitter (masquer l'icône)", "_Quit (hide icon)"),
                "application-exit-symbolic",
                true,
            ),
        ];

        Self {
            icon,
            status,
            tooltip_title,
            tooltip_lines,
            last_update: snap.last_update,
            menu,
        }
    }

    /// Tooltip body at unix time `now` (one piece of information per line).
    pub fn tooltip_body(&self, lang: Lang, now: u64, has_keyboard: bool) -> String {
        let mut lines = self.tooltip_lines.clone();
        if has_keyboard {
            lines.push(age_text(lang, self.last_update, now));
        }
        lines.join("\n")
    }

    pub fn entry(&self, id: i32) -> Option<&Entry> {
        self.menu.iter().find(|e| e.id == id)
    }
}

/// Plain-text block for the clipboard ("Copier les informations").
pub fn clipboard_text(snap: &Snapshot, charging: bool, lang: Lang, now: u64) -> String {
    let v = View::build(snap, charging, None, lang);
    let mut out = vec![v.tooltip_title.clone()];
    out.extend(v.tooltip_lines.iter().cloned());
    if let Some(k) = &snap.keyboard {
        if let Some(m) = k.device.mac.as_deref().filter(|m| !m.is_empty()) {
            out.push(format!("{} {m}", lang.t("MAC\u{a0}:", "MAC:")));
        }
        let fw = match lang {
            Lang::Fr => akm_core::firmware::summary_fr(&k.firmware),
            Lang::En => akm_core::firmware::summary_en(&k.firmware),
        };
        if let Some(fw) = fw {
            out.push(fw);
        }
    }
    if snap.keyboard.is_some() {
        out.push(age_text(lang, snap.last_update, now));
    }
    if let Some(e) = snap.last_error.as_deref().filter(|e| !e.is_empty()) {
        out.push(format!(
            "{} {e}",
            lang.t("Dernière erreur :", "Last error:")
        ));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::KbReport;

    fn snap(pct: Option<f64>, connected: bool, rssi: Option<i32>) -> Snapshot {
        let mut k = KbReport::default();
        k.battery.percentage_fine = pct;
        k.battery.voltage = Some(2.903);
        k.radio.rssi_dbm = rssi;
        k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
        k.device.name = Some("Clavier_test".into());
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        Snapshot {
            connected,
            keyboard: Some(k),
            last_update: 1_000,
            ..Default::default()
        }
    }

    #[test]
    fn locale() {
        assert_eq!(Lang::from_locale("fr_FR.UTF-8"), Lang::Fr);
        assert_eq!(Lang::from_locale("en_US.UTF-8"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }

    #[test]
    fn buckets_round_down() {
        assert_eq!(bucket(0.0, None), 0);
        assert_eq!(bucket(9.9, None), 0);
        assert_eq!(bucket(10.0, None), 10);
        assert_eq!(bucket(55.0, None), 50);
        assert_eq!(bucket(99.4, None), 90);
        assert_eq!(bucket(100.0, None), 100);
        assert_eq!(bucket(140.0, None), 100);
        assert_eq!(bucket(-3.0, None), 0);
    }

    #[test]
    fn buckets_have_hysteresis() {
        // 49-51 % around the 50 edge: no flapping.
        assert_eq!(bucket(51.0, None), 50);
        assert_eq!(bucket(49.0, Some(50)), 50);
        assert_eq!(bucket(48.0, Some(50)), 50);
        assert_eq!(bucket(47.9, Some(50)), 40);
        assert_eq!(bucket(51.0, Some(40)), 40);
        assert_eq!(bucket(52.0, Some(40)), 50);
        // Going up to 100 is always possible, staying at 100 down to 98.
        assert_eq!(bucket(100.0, Some(90)), 100);
        assert_eq!(bucket(98.0, Some(100)), 100);
        assert_eq!(bucket(97.0, Some(100)), 90);
        // Sequence 49, 51, 49, 51 from 50 never changes the icon.
        let mut b = None;
        let mut changes = 0;
        for p in [50.0, 49.0, 51.0, 49.0, 51.0, 49.5, 50.5] {
            let n = bucket(p, b);
            if b.is_some() && b != Some(n) {
                changes += 1;
            }
            b = Some(n);
        }
        assert_eq!(changes, 0);
    }

    #[test]
    fn icon_names_per_set() {
        let l = IconState::Level {
            bucket: 50,
            charging: false,
        };
        let c = IconState::Level {
            bucket: 0,
            charging: true,
        };
        assert_eq!(l.name(IconSet::Ours), "apihub-kb-battery-050-symbolic");
        assert_eq!(
            c.name(IconSet::Ours),
            "apihub-kb-battery-000-charging-symbolic"
        );
        assert_eq!(l.name(IconSet::Breeze), "battery-050-symbolic");
        assert_eq!(c.name(IconSet::Breeze), "battery-000-charging-symbolic");
        assert_eq!(l.name(IconSet::Adwaita), "battery-level-50-symbolic");
        assert_eq!(
            IconState::Level {
                bucket: 100,
                charging: false
            }
            .name(IconSet::Adwaita),
            "battery-level-100-symbolic"
        );
        assert_eq!(
            IconState::Caution.name(IconSet::Ours),
            "apihub-kb-battery-caution-symbolic"
        );
        assert_eq!(
            IconState::Caution.name(IconSet::Breeze),
            "battery-caution-symbolic"
        );
        assert_eq!(
            IconState::Disconnected.name(IconSet::Ours),
            "apihub-kb-disconnected-symbolic"
        );
        assert_eq!(
            IconState::Missing.name(IconSet::Ours),
            "apihub-kb-missing-symbolic"
        );
    }

    #[test]
    fn every_icon_of_our_set_is_shipped() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../icons/hicolor/scalable/status");
        let mut states = vec![
            IconState::Caution,
            IconState::Disconnected,
            IconState::Missing,
        ];
        for b in (0..=100).step_by(10) {
            for charging in [false, true] {
                states.push(IconState::Level {
                    bucket: b as u8,
                    charging,
                });
            }
        }
        for s in states {
            let f = dir.join(format!("{}.svg", s.name(IconSet::Ours)));
            assert!(f.is_file(), "missing {}", f.display());
        }
    }

    #[test]
    fn icon_state_choice() {
        assert_eq!(
            icon_state(&Snapshot::default(), false, None),
            IconState::Missing
        );
        assert_eq!(
            icon_state(&snap(Some(80.0), false, None), false, None),
            IconState::Disconnected
        );
        assert_eq!(
            icon_state(&snap(None, true, None), false, None),
            IconState::Missing
        );
        assert_eq!(
            icon_state(&snap(Some(10.0), true, None), false, None),
            IconState::Caution
        );
        assert_eq!(
            icon_state(&snap(Some(10.0), true, None), true, None),
            IconState::Level {
                bucket: 10,
                charging: true
            }
        );
        assert_eq!(
            icon_state(&snap(Some(99.0), true, None), false, None),
            IconState::Level {
                bucket: 90,
                charging: false
            }
        );
    }

    #[test]
    fn rssi_unknown_is_absent_never_127() {
        assert_eq!(rssi_text(Lang::Fr, None), None);
        assert_eq!(rssi_text(Lang::Fr, Some(127)), None);
        // #174: words first, raw value without unit; 0 is ideal, > 0 is legal.
        assert_eq!(
            rssi_text(Lang::Fr, Some(0)).unwrap(),
            "Signal\u{a0}: excellent (0)"
        );
        assert_eq!(
            rssi_text(Lang::En, Some(-48)).unwrap(),
            "Signal: weak (\u{2212}48)"
        );
        assert_eq!(
            rssi_text(Lang::Fr, Some(-4)).unwrap(),
            "Signal\u{a0}: bon (\u{2212}4)"
        );
        assert_eq!(
            rssi_text(Lang::En, Some(2)).unwrap(),
            "Signal: excellent (+2)"
        );
        for r in [0, -4, 2, -48] {
            assert!(!rssi_text(Lang::En, Some(r)).unwrap().contains("dBm"));
        }

        let v = View::build(&snap(Some(99.0), true, None), false, None, Lang::Fr);
        let body = v.tooltip_body(Lang::Fr, 1_012, true);
        assert!(!body.contains("127") && !body.contains("Signal"), "{body}");
        assert!(!v.entry(id::SIGNAL).unwrap().visible());
        let v = View::build(&snap(Some(99.0), true, Some(127)), false, None, Lang::Fr);
        assert!(!v.tooltip_body(Lang::Fr, 1_012, true).contains("127"));
    }

    #[test]
    fn rich_tooltip() {
        let mut s = snap(Some(99.0), true, Some(-3));
        s.remaining_display = Some("≈ 41 j".into());
        let v = View::build(&s, false, None, Lang::Fr);
        assert_eq!(
            v.tooltip_title,
            "Clavier_test \u{2014} 99\u{a0}%"
        );
        let body = v.tooltip_body(Lang::Fr, 1_012, true);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            lines,
            vec![
                "Apple Wireless Keyboard (A1314, aluminum, ISO) \u{b7} Clavier_test",
                "Indication du clavier : 99\u{a0}% \u{b7} 2,90\u{a0}V",
                "Signal\u{a0}: bon (\u{2212}3)",
                "Autonomie estimée : ≈ 41 j",
                "Mis à jour il y a 12\u{a0}s",
            ]
        );
        assert_eq!(v.status, Status::Active);

        let v = View::build(&s, false, None, Lang::En);
        let body = v.tooltip_body(Lang::En, 1_000 + 7_200, true);
        assert!(body.contains("Keyboard indication: 99% \u{b7} 2.90 V"), "{body}");
        assert!(body.ends_with("Updated 2\u{a0}h ago"), "{body}");
    }

    #[test]
    fn estimate_is_shown_next_to_the_keyboard_indication() {
        // #178: indication 62 %, estimate 30 % (20-40 %), alkaline.
        let mut s = snap(Some(62.0), true, Some(0));
        s.keyboard.as_mut().unwrap().battery.charge_estimate =
            akm_core::chemistry::estimate_charge(2460, akm_core::chemistry::Chemistry::Alkaline);
        let v = View::build(&s, false, None, Lang::Fr);
        let body = v.tooltip_body(Lang::Fr, 1_012, true);
        assert!(body.contains("Indication du clavier : 62\u{a0}%"), "{body}");
        assert!(
            body.contains("Estimation (alcaline) :\u{a0}\u{2248}\u{a0}30\u{a0}% (20 \u{e0} 40\u{a0}%)"),
            "{body}"
        );
        assert!(v.entry(id::ESTIMATE).unwrap().visible());
        let en = View::build(&s, false, None, Lang::En).tooltip_body(Lang::En, 1_012, true);
        assert!(en.contains("Estimate (alkaline): \u{2248} 30% (20 to 40%)"), "{en}");
        // Offline: no estimate line (stale voltage).
        s.connected = false;
        assert!(!View::build(&s, false, None, Lang::Fr).entry(id::ESTIMATE).unwrap().visible());
        // New batteries: said, no figure.
        let mut n = snap(Some(98.0), true, Some(0));
        n.keyboard.as_mut().unwrap().battery.new_batteries = true;
        let b = View::build(&n, false, None, Lang::Fr).tooltip_body(Lang::Fr, 1_012, true);
        assert!(b.contains("Piles neuves"), "{b}");
        // No estimate (undeclared chemistry): nothing invented.
        let none = View::build(&snap(Some(62.0), true, Some(0)), false, None, Lang::Fr);
        assert!(!none.entry(id::ESTIMATE).unwrap().visible());
    }

    #[test]
    fn tooltip_offline_and_missing() {
        let v = View::build(&snap(Some(64.0), false, Some(-3)), false, None, Lang::Fr);
        assert_eq!(
            v.tooltip_title,
            "Clavier_test \u{2014} hors ligne"
        );
        let body = v.tooltip_body(Lang::Fr, 1_000, true);
        assert!(body.contains("Dernière valeur : 64\u{a0}%"));
        assert!(!body.contains("Signal"), "no stale RSSI when offline");

        let s = Snapshot {
            kb_error: Some("aucun clavier Apple appairé".into()),
            ..Default::default()
        };
        let v = View::build(&s, false, None, Lang::Fr);
        assert_eq!(v.tooltip_title, "Clavier Apple");
        assert_eq!(
            v.tooltip_body(Lang::Fr, 5, false),
            "aucun clavier Apple appairé"
        );
        assert_eq!(v.icon, IconState::Missing);
    }

    #[test]
    fn critical_needs_attention_and_alert() {
        let v = View::build(&snap(Some(8.0), true, None), false, None, Lang::Fr);
        assert_eq!(v.status, Status::NeedsAttention);
        assert_eq!(v.icon, IconState::Caution);
        assert_eq!(
            v.entry(id::BATTERY).unwrap().get("disposition"),
            Some(&Prop::Str("alert".into()))
        );
        let v = View::build(&snap(Some(18.0), true, None), false, None, Lang::Fr);
        assert_eq!(v.status, Status::Active);
        assert_eq!(
            v.entry(id::BATTERY).unwrap().get("disposition"),
            Some(&Prop::Str("warning".into()))
        );
        let v = View::build(&snap(Some(8.0), true, None), true, None, Lang::Fr);
        assert_eq!(v.status, Status::Active, "charging is not an alert");
    }

    #[test]
    fn menu_is_complete_and_stable() {
        let v = View::build(&snap(Some(99.0), true, Some(0)), false, None, Lang::Fr);
        let ids: Vec<i32> = v.menu.iter().map(|e| e.id).collect();
        assert_eq!(ids, id::ALL.to_vec());
        let v2 = View::build(&snap(Some(50.0), false, None), false, None, Lang::Fr);
        let ids2: Vec<i32> = v2.menu.iter().map(|e| e.id).collect();
        assert_eq!(ids, ids2, "ids never change, only visibility");
        // Data underscores are escaped (mnemonics), actions carry one.
        assert_eq!(
            v.entry(id::HEADER).unwrap().get("label"),
            Some(&Prop::Str("Clavier__test".into()))
        );
        assert!(v
            .entry(id::BATTERY)
            .unwrap()
            .get("accessible-desc")
            .is_some_and(|d| *d == Prop::Str("Indication du clavier 99 pour cent, 2,90 volts".into())));
        assert_eq!(
            v2.entry(id::REFRESH).unwrap().get("enabled"),
            Some(&Prop::Bool(false))
        );
        assert_eq!(esc("a_b"), "a__b");
    }

    #[test]
    fn clipboard_block() {
        let t = clipboard_text(&snap(Some(99.0), true, Some(-3)), false, Lang::En, 1_030);
        assert!(t.starts_with("Clavier_test \u{2014} 99%"), "{t}");
        assert!(t.contains("MAC: AA:BB:CC:DD:EE:F1"), "{t}");
        assert!(t.contains("Updated 30\u{a0}s ago"), "{t}");
        let t = clipboard_text(&snap(Some(99.0), true, None), false, Lang::Fr, 1_030);
        assert!(t.contains("MAC\u{a0}: AA:BB:CC:DD:EE:F1"), "{t}");
        assert!(!t.contains("127"));
    }

    #[test]
    fn alias_wins_everywhere_and_rename_is_offered() {
        let mut s = snap(Some(99.0), true, Some(-3));
        s.keyboard.as_mut().unwrap().device.alias = Some("Clavier de alice #1".into());
        let v = View::build(&s, false, None, Lang::En);
        assert_eq!(v.tooltip_title, "Clavier de alice #1 \u{2014} 99%");
        assert_eq!(
            v.tooltip_lines[0],
            "Apple Wireless Keyboard (A1314, aluminum, ISO) \u{b7} Clavier de alice #1"
        );
        assert_eq!(
            v.entry(id::HEADER).unwrap().get("label"),
            Some(&Prop::Str("Clavier de alice #1".into()))
        );
        let r = v.entry(id::RENAME).unwrap();
        assert_eq!(r.get("label"), Some(&Prop::Str("Re_name keyboard\u{2026}".into())));
        assert_eq!(r.get("enabled"), None, "enabled while a keyboard is known");
        let none = View::build(&Snapshot::default(), false, None, Lang::Fr);
        assert_eq!(none.entry(id::RENAME).unwrap().get("enabled"), Some(&Prop::Bool(false)));
    }

    #[test]
    fn tooltip_has_one_firmware_line_and_the_labelled_apple_display() {
        let mut s = snap(Some(62.0), true, Some(0));
        {
            let k = s.keyboard.as_mut().unwrap();
            k.firmware.version = Some("0x0050".into());
            akm_core::firmware::assess_report(Some(0x0256), &mut k.firmware);
            k.battery.apple_display_pct = Some(100.0);
        }
        let fr = View::build(&s, false, None, Lang::Fr).tooltip_body(Lang::Fr, 1_012, true);
        assert!(
            fr.contains("Firmware : 0x0050 \u{2014} \u{e0} jour (derni\u{e8}re version publique connue d'Apple)"),
            "{fr}"
        );
        assert_eq!(fr.matches("Firmware").count(), 1, "one line: {fr}");
        assert!(fr.contains("Affichage Apple :"), "{fr}");
        let en = View::build(&s, false, None, Lang::En).tooltip_body(Lang::En, 1_012, true);
        assert!(en.contains("Firmware: 0x0050 - up to date"), "{en}");
        assert!(en.contains("Apple display: 100%"), "{en}");
        // The indication line is still there, and still first.
        assert!(en.find("Keyboard indication").unwrap() < en.find("Apple display").unwrap());
        // Not read yet / not applicable: nothing invented.
        let none = View::build(&snap(Some(62.0), true, Some(0)), false, None, Lang::En)
            .tooltip_body(Lang::En, 1_012, true);
        assert!(!none.contains("Firmware") && !none.contains("Apple display"), "{none}");
        // Clipboard text carries the same sentence.
        assert!(clipboard_text(&s, false, Lang::Fr, 1_012).contains("\u{e0} jour"));
    }
}
