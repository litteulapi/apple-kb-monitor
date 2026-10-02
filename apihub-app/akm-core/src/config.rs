//! `config.toml` of the daemon (`$XDG_CONFIG_HOME/apple-kb-monitor/config.toml`).
//!
//! ```toml
//! [alerts]
//! thresholds = [30, 15, 5]   # % ; one notification per threshold crossed
//! hysteresis = 3             # re-armed at threshold + 3 points
//! critical = 5               # thresholds <= 5 are sent as "critical"
//! enabled = true
//!
//! [battery]
//! chemistry = "alkaline"     # "alkaline" | "nimh" | "lithium" | "unknown"
//!
//! [notifications]
//! connection = true          # "disconnected" / "reconnected (N %)", low urgency
//! battery_replaced = true    # "new batteries detected"
//! defer_to_powerdevil = true # PowerDevil already warns about this keyboard: one distinct reminder only (#254)
//! battery_advice = true     # "batteries changed too often" after two sets under 30 days (#108)
//! quiet_hours = "22:00-07:00" # non-critical notifications held until the range ends; "" = none (#91)
//!
//! [osd]
//! fn_mode = true             # Plasma OSD when the Fn mode changes (#100)
//! caps_lock = true           # Plasma OSD "Caps Lock on / off" when the key is pressed (#101)
//!
//! [usage]
//! active_time = false        # count the minutes per day the keyboard is used; no key is ever recorded (#109)
//!
//! [display]
//! apple_percent = true       # also show the percentage "as macOS shows it" (#213)
//!
//! [apple]
//! will_shutdown = true       # tell the keyboard once, at shutdown / restart, what macOS tells it (#191)
//! disconnect_on_breaker = true  # after 3 unanswered requests, have BlueZ disconnect it, as macOS (#251)
//! allow_device_name_write = false  # OBSOLETE: read, ignored (was a lock of `akmctl rename --device-name`, #248)
//! ```
//!
//! The file is read by the `toml` crate and typed through `serde` (#12): any
//! valid TOML document is understood (an UTF-8 BOM is skipped). Unknown keys
//! and mistyped values are reported as warnings and otherwise ignored; a
//! missing file means the defaults. A file that is NOT valid TOML is not
//! thrown away as a whole: it is read again statement by statement, each
//! `key = value` going through the same crate, so that one bad line costs one
//! warning and one default, never the whole configuration.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use toml::de::{DeTable, DeValue, ValueDeserializer};
use toml::Spanned;

use crate::alerts::{AlertConfig, DEFAULT_HYSTERESIS};
use crate::chemistry::Chemistry;
use crate::quiet::QuietHours;

const REL_PATH: &str = "apple-kb-monitor/config.toml";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub alerts: AlertConfig,
    pub alerts_enabled: bool,
    pub notify_connection: bool,
    pub notify_battery_replaced: bool,
    /// "Batteries changed too often" notification, once, when the second set
    /// in a row is replaced within 30 days (#108). Default on.
    pub notify_battery_advice: bool,
    /// When KDE PowerDevil already raises its low-battery notification for
    /// this keyboard, our percentage alerts shrink to one distinct reminder
    /// ("estimate from your batteries", #254). Default **on**.
    pub defer_to_powerdevil: bool,
    /// Hours during which a notification that is not critical is held and
    /// shown when they end (`"22:00-07:00"`, several separated by commas;
    /// #91). Default: none.
    pub quiet_hours: QuietHours,
    /// Plasma OSD when `hid_apple.fnmode` changes (`[osd] fn_mode`, #100).
    pub osd_fn_mode: bool,
    /// Plasma OSD when Caps Lock is pressed (`[osd] caps_lock`, #101).
    pub osd_caps_lock: bool,
    /// Count the active minutes per day from the mere presence of input
    /// reports (`[usage] active_time`, #109). Default **off**.
    pub usage_active_time: bool,
    /// Declared chemistry of the batteries (#178), default alkaline.
    pub chemistry: Chemistry,
    /// Show the percentage as macOS displays it, labelled "Apple display"
    /// (#213). Pure host-side computation, default on.
    pub apple_percent: bool,
    /// Send `WillShutdown` (Feature `0x40`, the id alone) to the keyboard when
    /// the computer shuts down or restarts, as macOS does at every shutdown
    /// (#191). Default **on**: it is what Apple sends; `false` sends nothing.
    pub will_shutdown: bool,
    /// After the 3rd unanswered request in a row (Apple's breaker), ask BlueZ
    /// once to disconnect the keyboard (`Device1.Disconnect`), as macOS asks
    /// bluetoothd (#251). Default **on**; `false` only stops the requests.
    pub disconnect_on_breaker: bool,
    /// OBSOLETE (#248): was a lock of `akmctl rename --device-name`. Still
    /// read so that an existing `config.toml` raises no warning; nothing uses
    /// it any more (the write asks one confirmation, or takes `--yes`).
    pub allow_device_name_write: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alerts: AlertConfig::default(),
            alerts_enabled: true,
            notify_connection: true,
            notify_battery_replaced: true,
            notify_battery_advice: true,
            defer_to_powerdevil: true,
            quiet_hours: QuietHours::none(),
            osd_fn_mode: true,
            osd_caps_lock: true,
            usage_active_time: false,
            chemistry: Chemistry::default(),
            apple_percent: true,
            will_shutdown: true,
            disconnect_on_breaker: true,
            allow_device_name_write: false,
        }
    }
}

/// `$XDG_CONFIG_HOME/apple-kb-monitor/config.toml` (fallback `~/.config`).
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/etc"))
        .join(REL_PATH)
}

/// `line` without its trailing `#` comment; a `#` inside a `"..."` string stays.
/// Only used by the statement-by-statement reading of an invalid document.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Sections written by other programs that read the same `config.toml` (the
/// former MQTT/DDC screen bridge). Silently skipped, sub-tables included.
const FOREIGN_SECTIONS: [&str; 4] = ["ddc", "mqtt", "monitor", "brightness"];

fn is_foreign(section: &str) -> bool {
    let root = section.split('.').next().unwrap_or("").trim();
    FOREIGN_SECTIONS.contains(&root)
}

/// `v` as a `T`, through serde: strict (an integer is not a boolean, a string
/// is not a number); only an integer is also accepted where a float is asked.
fn typed<T: DeserializeOwned>(v: &Spanned<DeValue<'_>>) -> Option<T> {
    T::deserialize(ValueDeserializer::from(v.clone())).ok()
}

/// What the document says so far, plus the warnings (with their line).
struct Reader {
    cfg: Config,
    warn: Vec<(usize, String)>,
    thresholds: Option<Vec<u8>>,
    hysteresis: f64,
    critical: u8,
}

impl Reader {
    fn new() -> Self {
        Self {
            cfg: Config::default(),
            warn: Vec::new(),
            thresholds: None,
            hysteresis: DEFAULT_HYSTERESIS,
            critical: 5,
        }
    }

    fn warn(&mut self, line: usize, msg: impl fmt::Display) {
        self.warn.push((line, format!("line {line}: {msg}")));
    }

    /// One `key = value` of `[section]`, read on `line`.
    fn apply(&mut self, section: &str, key: &str, v: &Spanned<DeValue<'_>>, line: usize) {
        // Sections that belong to another program sharing this file (the
        // former screen/MQTT bridge, now lg-ddc-control): not ours, so no warning.
        if is_foreign(section) {
            return;
        }
        let flag = |slot: &mut bool| typed::<bool>(v).map(|b| *slot = b).is_some();
        let known = match (section, key) {
            ("alerts", "thresholds") => match typed::<Vec<i64>>(v) {
                Some(list) => {
                    let pct = |x: i64| u8::try_from(x).ok().filter(|p| (1..=99).contains(p));
                    let ok: Vec<u8> = list.iter().filter_map(|&x| pct(x)).collect();
                    if ok.len() != list.len() {
                        self.warn(line, "thresholds must be in 1..=99");
                    }
                    if ok.is_empty() {
                        self.warn(line, "thresholds is empty: no battery alert will be sent");
                    }
                    self.thresholds = Some(ok);
                    true
                }
                None => false,
            },
            ("alerts", "hysteresis") => match typed::<f64>(v).filter(|f| f.is_finite()) {
                Some(f) => {
                    self.hysteresis = f;
                    true
                }
                None => false,
            },
            ("alerts", "critical") => match typed::<i64>(v) {
                Some(i) => {
                    match u8::try_from(i) {
                        Ok(c) if c <= 99 => self.critical = c,
                        _ => self.warn(line, "critical must be in 0..=99"),
                    }
                    true
                }
                None => false,
            },
            ("alerts", "enabled") => flag(&mut self.cfg.alerts_enabled),
            ("battery", "chemistry") => match typed::<String>(v) {
                Some(name) => {
                    match Chemistry::parse(&name) {
                        Some(c) => self.cfg.chemistry = c,
                        None => self.warn(
                            line,
                            "chemistry must be alkaline, nimh, lithium or unknown (alkaline kept)",
                        ),
                    }
                    true
                }
                None => false,
            },
            ("osd", "fn_mode") => flag(&mut self.cfg.osd_fn_mode),
            ("osd", "caps_lock") => flag(&mut self.cfg.osd_caps_lock),
            ("usage", "active_time") => flag(&mut self.cfg.usage_active_time),
            ("display", "apple_percent") => flag(&mut self.cfg.apple_percent),
            ("apple", "will_shutdown") => flag(&mut self.cfg.will_shutdown),
            ("apple", "disconnect_on_breaker") => flag(&mut self.cfg.disconnect_on_breaker),
            ("apple", "allow_device_name_write") => flag(&mut self.cfg.allow_device_name_write),
            ("notifications", "connection") => flag(&mut self.cfg.notify_connection),
            ("notifications", "battery_replaced") => flag(&mut self.cfg.notify_battery_replaced),
            ("notifications", "battery_advice") => flag(&mut self.cfg.notify_battery_advice),
            ("notifications", "defer_to_powerdevil") => flag(&mut self.cfg.defer_to_powerdevil),
            ("notifications", "quiet_hours") => match typed::<String>(v) {
                Some(text) => {
                    match QuietHours::parse(&text) {
                        Ok(q) => self.cfg.quiet_hours = q,
                        Err(e) => self.warn(
                            line,
                            format_args!(
                                "quiet_hours must be \"HH:MM-HH:MM\" ({e}); no quiet hours"
                            ),
                        ),
                    }
                    true
                }
                None => false,
            },
            _ => false,
        };
        if !known {
            self.warn(
                line,
                format_args!("unknown or mistyped key [{section}] {key}"),
            );
        }
    }

    /// A valid TOML document, as the `toml` crate read it.
    fn document(&mut self, content: &str, doc: &DeTable<'_>) {
        let line_of = |at: usize| content[..at.min(content.len())].matches('\n').count() + 1;
        for (name, item) in doc {
            match item.get_ref() {
                DeValue::Table(section) => {
                    for (key, value) in section {
                        let line = line_of(key.span().start);
                        self.apply(name.get_ref(), key.get_ref(), value, line);
                    }
                }
                // A key outside any section (or an array of tables): section
                // "" has no key of ours.
                _ => {
                    let line = line_of(name.span().start);
                    self.warn(
                        line,
                        format_args!("unknown or mistyped key [] {}", name.get_ref()),
                    );
                }
            }
        }
    }

    /// A document the `toml` crate refuses (a stray word, a section opened
    /// twice, an unquoted value...): read statement by statement, every
    /// `key = value` still going through the crate, so that only the bad
    /// lines are lost.
    fn statements(&mut self, content: &str) {
        let mut section = String::new();
        let mut lines = content.lines().enumerate();
        while let Some((n, raw)) = lines.next() {
            let mut line = strip_comment(raw).trim().to_string();
            if line.is_empty() {
                continue;
            }
            // Multi-line array: accumulate until the closing bracket.
            if line.split_once('=').is_some_and(|(_, v)| {
                let v = v.trim();
                v.starts_with('[') && !v.ends_with(']')
            }) {
                let mut closed = false;
                for (_, more) in lines.by_ref() {
                    line.push(' ');
                    line.push_str(strip_comment(more).trim());
                    if line.ends_with(']') {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    self.warn(n + 1, "unterminated array");
                    continue;
                }
            }
            let line = line.as_str();
            if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                section = name.trim().to_string();
                continue;
            }
            if is_foreign(&section) {
                continue;
            }
            let Some((k, _)) = line.split_once('=') else {
                self.warn(n + 1, "not a `key = value` line");
                continue;
            };
            match DeTable::parse(line) {
                Ok(table) => {
                    for (key, value) in table.get_ref() {
                        self.apply(&section, key.get_ref(), value, n + 1);
                    }
                }
                Err(_) => self.warn(n + 1, format_args!("unreadable value for {}", k.trim())),
            }
        }
    }

    fn finish(mut self) -> (Config, Vec<String>) {
        self.cfg.alerts = AlertConfig::new(
            self.thresholds
                .unwrap_or_else(|| self.cfg.alerts.thresholds().to_vec()),
            self.hysteresis,
            self.critical,
        );
        // Stable: two warnings of one line keep their order.
        self.warn.sort_by_key(|(line, _)| *line);
        (self.cfg, self.warn.into_iter().map(|(_, m)| m).collect())
    }
}

/// Parse the file content. Returns the config and human-readable warnings.
pub fn parse(content: &str) -> (Config, Vec<String>) {
    let content = content.trim_start_matches('\u{feff}');
    let mut reader = Reader::new();
    match DeTable::parse(content) {
        Ok(doc) => reader.document(content, doc.get_ref()),
        Err(_) => reader.statements(content),
    }
    reader.finish()
}

/// Load `path`; a missing file gives the defaults without warning.
pub fn load(path: &Path) -> (Config, Vec<String>) {
    match std::fs::read_to_string(path) {
        Ok(c) => parse(&c),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
        Err(e) => (
            Config::default(),
            vec![format!("{}: {e}; using defaults", path.display())],
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn foreign_sections_of_other_programs_are_ignored_silently() {
        // The former screen/MQTT bridge writes these into the same file.
        let (_, warn) = parse(
            "[ddc]\nbus = \"/dev/i2c-3\"\n[mqtt]\nbroker = \"x\"\nport = 1883\n[monitor]\nmodel = \"m\"\n[brightness]\nmin = 2\nlamp_entity = \"l\"\n[alerts]\nenabled = true\n",
        );
        assert!(warn.is_empty(), "{warn:?}");
        // A typo in one of OUR sections is still reported.
        let (_, warn) = parse("[apple]\nwill_shutdwn = true\n");
        assert_eq!(warn.len(), 1, "{warn:?}");
        // `call_audio_hint` (0x4A) is deliberately not implemented
        // (docs/PARITE-APPLE.md): the key is reported, never silently taken.
        let (_, warn) = parse("[apple]\ncall_audio_hint = \"auto\"\n");
        assert_eq!(warn.len(), 1, "{warn:?}");
        assert!(warn[0].contains("call_audio_hint"), "{warn:?}");
    }

    use super::*;

    #[test]
    fn documented_example_parses() {
        let (c, w) = parse(
            "# comment\n[alerts]\nthresholds = [30, 15, 5]  # %\nhysteresis = 3\ncritical = 5\nenabled = true\n\n[notifications]\nconnection = false\nbattery_replaced = true\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.alerts.thresholds(), &[30, 15, 5]);
        assert_eq!(c.alerts.hysteresis, 3.0);
        assert!(c.alerts_enabled && !c.notify_connection && c.notify_battery_replaced);
    }

    #[test]
    fn apple_percent_option() {
        assert!(Config::default().apple_percent);
        let (c, w) = parse("[display]\napple_percent = false\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!c.apple_percent);
        let (c, w) = parse("[display]\napple_percent = 3\n");
        assert_eq!(w.len(), 1);
        assert!(c.apple_percent, "a mistyped value keeps the default");
    }

    #[test]
    fn defer_to_powerdevil_defaults_to_on() {
        assert!(Config::default().defer_to_powerdevil);
        let (c, w) = parse("[notifications]\ndefer_to_powerdevil = false\n");
        assert!(w.is_empty() && !c.defer_to_powerdevil);
        let (c, w) = parse("[notifications]\ndefer_to_powerdevil = 3\n");
        assert_eq!(w.len(), 1);
        assert!(c.defer_to_powerdevil, "a mistyped value keeps the default");
    }

    #[test]
    fn quiet_hours_are_off_by_default_and_read_as_a_range() {
        // #91
        assert!(Config::default().quiet_hours.is_empty());
        assert!(parse("[notifications]\nconnection = true\n")
            .0
            .quiet_hours
            .is_empty());
        let (c, w) = parse("[notifications]\nquiet_hours = \"22:00-07:00\"\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.quiet_hours.contains(23 * 60) && c.quiet_hours.contains(6 * 60));
        assert!(!c.quiet_hours.contains(12 * 60));
        let (c, w) = parse("[notifications]\nquiet_hours = \"12:00-13:00, 22:00-07:00\"\n");
        assert!(w.is_empty() && c.quiet_hours.contains(12 * 60 + 30));
        // Explicitly none.
        let (c, w) = parse("[notifications]\nquiet_hours = \"\"\n");
        assert!(w.is_empty() && c.quiet_hours.is_empty());
        // A malformed range: one warning naming the format, no quiet hours
        // (a notification is never held by a value nobody understood).
        for bad in ["\"22h-7h\"", "\"25:00-07:00\"", "\"08:00-08:00\""] {
            let (c, w) = parse(&format!("[notifications]\nquiet_hours = {bad}\n"));
            assert_eq!(w.len(), 1, "{bad}: {w:?}");
            assert!(w[0].contains("HH:MM-HH:MM"), "{w:?}");
            assert!(c.quiet_hours.is_empty(), "{bad}");
        }
        // Not a string: mistyped.
        let (c, w) = parse("[notifications]\nquiet_hours = 22\n");
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("mistyped") && c.quiet_hours.is_empty());
    }

    #[test]
    fn battery_advice_notification_defaults_to_on() {
        // #108
        assert!(Config::default().notify_battery_advice);
        let (c, w) = parse("[notifications]\nbattery_advice = false\n");
        assert!(w.is_empty() && !c.notify_battery_advice);
        let (c, w) = parse("[notifications]\nbattery_advice = \"no\"\n");
        assert_eq!(w.len(), 1);
        assert!(
            c.notify_battery_advice,
            "a mistyped value keeps the default"
        );
    }

    #[test]
    fn usage_statistics_are_off_by_default() {
        // #109
        assert!(!Config::default().usage_active_time);
        assert!(!parse("[usage]\n").0.usage_active_time);
        let (c, w) = parse("[usage]\nactive_time = true\n");
        assert!(w.is_empty() && c.usage_active_time);
        let (c, w) = parse("[usage]\nactive_time = 1\nkeys = true\n");
        assert_eq!(w.len(), 2, "mistyped value, and no other key exists: {w:?}");
        assert!(!c.usage_active_time);
    }

    #[test]
    fn both_osds_are_on_by_default_and_can_be_turned_off() {
        // #100, #101
        let d = Config::default();
        assert!(d.osd_fn_mode && d.osd_caps_lock);
        let (c, w) = parse("[osd]\nfn_mode = false\n");
        assert!(w.is_empty() && !c.osd_fn_mode && c.osd_caps_lock);
        let (c, w) = parse("[osd]\ncaps_lock = false\n");
        assert!(w.is_empty() && c.osd_fn_mode && !c.osd_caps_lock);
        let (c, w) = parse("[osd]\ncaps_lock = \"off\"\nvolume = true\n");
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(c.osd_caps_lock, "a mistyped value keeps the default");
    }

    #[test]
    fn will_shutdown_option_defaults_to_on() {
        // #191: Apple sends WillShutdown at every shutdown, so the default is on.
        assert!(Config::default().will_shutdown);
        assert!(parse("").0.will_shutdown);
        let (c, w) = parse("[apple]\nwill_shutdown = false  # keep the keyboard untouched\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!c.will_shutdown);
        let (c, w) = parse("[apple]\nwill_shutdown = true\n");
        assert!(w.is_empty() && c.will_shutdown);
        let (c, w) = parse("[apple]\nwill_shutdown = \"no\"\n");
        assert_eq!(w.len(), 1);
        assert!(c.will_shutdown, "a mistyped value keeps the default");
        let (_, w) = parse("[apple]\nwill_shutdown = true\nname_change = true\n");
        assert_eq!(w.len(), 1, "no other [apple] key exists: {w:?}");
    }

    #[test]
    fn disconnect_on_breaker_defaults_to_on() {
        // #251: macOS has bluetoothd disconnect the keyboard after 3 timeouts.
        assert!(Config::default().disconnect_on_breaker);
        assert!(parse("[apple]\n").0.disconnect_on_breaker);
        let (c, w) = parse("[apple]\ndisconnect_on_breaker = false\n");
        assert!(w.is_empty() && !c.disconnect_on_breaker && c.will_shutdown);
        let (c, w) = parse("[apple]\ndisconnect_on_breaker = 0\n");
        assert_eq!(w.len(), 1);
        assert!(
            c.disconnect_on_breaker,
            "a mistyped value keeps the default"
        );
    }

    #[test]
    fn the_obsolete_allow_device_name_write_key_is_still_read_without_a_warning() {
        // #248: obsolete, ignored by every command; a boolean raises no warning.
        assert!(!Config::default().allow_device_name_write);
        assert!(!parse("").0.allow_device_name_write);
        assert!(
            !parse("[apple]\nwill_shutdown = true\n")
                .0
                .allow_device_name_write
        );
        let (c, w) =
            parse("[apple]\nallow_device_name_write = true  # left over from an older version\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.allow_device_name_write);
        let (c, w) = parse("[apple]\nallow_device_name_write = false\n");
        assert!(w.is_empty() && !c.allow_device_name_write);
        // A mistyped value, a string, 1, "yes", or the key in another section: still off.
        for bad in [
            "[apple]\nallow_device_name_write = 1\n",
            "[apple]\nallow_device_name_write = \"true\"\n",
            "[apple]\nallow_device_name_write = yes\n",
            "[display]\nallow_device_name_write = true\n",
            "allow_device_name_write = true\n",
        ] {
            let (c, w) = parse(bad);
            assert!(!c.allow_device_name_write, "{bad:?}");
            assert_eq!(w.len(), 1, "{bad:?}: {w:?}");
        }
    }

    #[test]
    fn custom_values_and_defaults() {
        let (c, w) = parse("[alerts]\nthresholds=[20,10]\nhysteresis = 4.5\ncritical = 10\n");
        assert!(w.is_empty());
        assert_eq!(c.alerts.thresholds(), &[20, 10]);
        assert_eq!(c.alerts.hysteresis, 4.5);
        assert_eq!(c.alerts.urgency(10), crate::alerts::Urgency::Critical);
        assert_eq!(parse("").0, Config::default());
        assert_eq!(Config::default().alerts.thresholds(), &[30, 15, 5]);
    }

    #[test]
    fn bad_lines_are_warnings_not_errors() {
        let (c, w) = parse(
            "[alerts]\nthresholds = [30, 150, x]\nnope\nhysteresis = \"3\"\n[other]\nfoo = 1\n[alerts]\ncritical = 300\n",
        );
        assert_eq!(w.len(), 5, "{w:?}");
        assert_eq!(c.alerts, AlertConfig::default(), "unreadable array ignored");
        let (c, w) = parse("[alerts]\nthresholds = [30, 150]\n");
        assert_eq!(w.len(), 1);
        assert_eq!(c.alerts.thresholds(), &[30]);
    }

    #[test]
    fn multiline_array_bom_and_comments() {
        // #167: canonical multi-line TOML array and BOM.
        let (c, w) = parse("[alerts]\nthresholds = [\n  40,\n  20, # second\n]\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.alerts.thresholds(), &[40, 20]);
        let (c, w) = parse("\u{feff}[alerts]\nthresholds = [25]\nenabled = false\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.alerts.thresholds(), &[25]);
        assert!(!c.alerts_enabled);
        // closing bracket on the last value's line, comment after a string with a quote
        let (c, w) = parse("[alerts]\nthresholds = [\n40,\n20]\n");
        assert!(w.is_empty() && c.alerts.thresholds() == [40, 20], "{w:?}");
        assert_eq!(strip_comment("a = \"x#y\" # c"), "a = \"x#y\" ");
        let (_, w) = parse("[alerts]\nthresholds = [\n40,\n");
        assert_eq!(w, vec!["line 2: unterminated array".to_string()]);
    }

    #[test]
    fn empty_thresholds_warns() {
        let (c, w) = parse("[alerts]\nthresholds = []\n");
        assert!(c.alerts.thresholds().is_empty());
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("empty"));
    }

    #[test]
    fn battery_chemistry_is_read_and_defaults_to_alkaline() {
        // #178
        assert_eq!(Config::default().chemistry, Chemistry::Alkaline);
        let (c, w) = parse("[battery]\nchemistry = \"nimh\"  # Eneloop\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.chemistry, Chemistry::Nimh);
        let (c, _) = parse("[battery]\nchemistry = \"Unknown\"\n");
        assert_eq!(c.chemistry, Chemistry::Unknown);
        let (c, w) = parse("[battery]\nchemistry = \"zinc\"\n");
        assert_eq!(
            c.chemistry,
            Chemistry::Alkaline,
            "invalid value: default kept"
        );
        assert_eq!(w.len(), 1, "{w:?}");
        let (_, w) = parse("[battery]\nchemistry = 3\n");
        assert_eq!(w.len(), 1, "{w:?}");
    }

    /// The hand-written reader that `toml` + `serde` replaced (#12), kept
    /// verbatim as the oracle of `same_result_as_the_former_hand_written_reader`.
    mod legacy {
        use super::super::{AlertConfig, Chemistry, Config, DEFAULT_HYSTERESIS, FOREIGN_SECTIONS};

        #[derive(Debug, Clone, PartialEq)]
        enum Val {
            Int(i64),
            Float(f64),
            Bool(bool),
            Ints(Vec<i64>),
            Str(String),
        }

        fn parse_value(v: &str) -> Option<Val> {
            let v = v.trim();
            if v == "true" || v == "false" {
                return Some(Val::Bool(v == "true"));
            }
            if let Some(inner) = v.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                let items: Option<Vec<i64>> = inner
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.replace('_', "").parse().ok())
                    .collect();
                return items.map(Val::Ints);
            }
            if let Some(s) = v
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .filter(|s| !s.contains('"'))
            {
                return Some(Val::Str(s.to_string()));
            }
            let n = v.replace('_', "");
            n.parse::<i64>().map(Val::Int).ok().or_else(|| {
                n.parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite())
                    .map(Val::Float)
            })
        }

        /// `line` without its trailing `#` comment; a `#` inside a `"..."` string stays.
        fn strip_comment(line: &str) -> &str {
            let mut in_str = false;
            for (i, c) in line.char_indices() {
                match c {
                    '"' => in_str = !in_str,
                    '#' if !in_str => return &line[..i],
                    _ => {}
                }
            }
            line
        }

        pub fn parse(content: &str) -> (Config, Vec<String>) {
            let mut cfg = Config::default();
            let mut warn = Vec::new();
            let mut section = String::new();
            let mut thresholds: Option<Vec<u8>> = None;
            let mut hysteresis = DEFAULT_HYSTERESIS;
            let mut critical = 5u8;
            let content = content.trim_start_matches('\u{feff}');
            let mut lines = content.lines().enumerate();
            while let Some((n, raw)) = lines.next() {
                let mut line = strip_comment(raw).trim().to_string();
                if line.is_empty() {
                    continue;
                }
                // Multi-line array: accumulate until the closing bracket.
                if line.split_once('=').is_some_and(|(_, v)| {
                    let v = v.trim();
                    v.starts_with('[') && !v.ends_with(']')
                }) {
                    let mut closed = false;
                    for (_, more) in lines.by_ref() {
                        line.push(' ');
                        line.push_str(strip_comment(more).trim());
                        if line.ends_with(']') {
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        warn.push(format!("line {}: unterminated array", n + 1));
                        continue;
                    }
                }
                let line = line.as_str();
                if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    section = name.trim().to_string();
                    continue;
                }
                let Some((k, v)) = line.split_once('=') else {
                    warn.push(format!("line {}: not a `key = value` line", n + 1));
                    continue;
                };
                let key = k.trim();
                let Some(val) = parse_value(v) else {
                    warn.push(format!("line {}: unreadable value for {key}", n + 1));
                    continue;
                };
                let pct = |x: i64| u8::try_from(x).ok().filter(|p| (1..=99).contains(p));
                match (section.as_str(), key, val) {
                    ("alerts", "thresholds", Val::Ints(v)) => {
                        let ok: Vec<u8> = v.iter().filter_map(|&x| pct(x)).collect();
                        if ok.len() != v.len() {
                            warn.push(format!("line {}: thresholds must be in 1..=99", n + 1));
                        }
                        if ok.is_empty() {
                            warn.push(format!(
                                "line {}: thresholds is empty: no battery alert will be sent",
                                n + 1
                            ));
                        }
                        thresholds = Some(ok);
                    }
                    ("alerts", "hysteresis", Val::Int(i)) => hysteresis = i as f64,
                    ("alerts", "hysteresis", Val::Float(f)) => hysteresis = f,
                    ("alerts", "critical", Val::Int(i)) => match u8::try_from(i) {
                        Ok(c) if c <= 99 => critical = c,
                        _ => warn.push(format!("line {}: critical must be in 0..=99", n + 1)),
                    },
                    ("alerts", "enabled", Val::Bool(b)) => cfg.alerts_enabled = b,
                    ("battery", "chemistry", Val::Str(v)) => match Chemistry::parse(&v) {
                        Some(c) => cfg.chemistry = c,
                        None => warn.push(format!(
                            "line {}: chemistry must be alkaline, nimh, lithium or unknown (alkaline kept)",
                            n + 1
                        )),
                    },
                    ("display", "apple_percent", Val::Bool(b)) => cfg.apple_percent = b,
                    ("apple", "will_shutdown", Val::Bool(b)) => cfg.will_shutdown = b,
                    ("apple", "disconnect_on_breaker", Val::Bool(b)) => cfg.disconnect_on_breaker = b,
                    ("apple", "allow_device_name_write", Val::Bool(b)) => cfg.allow_device_name_write = b,
                    ("notifications", "connection", Val::Bool(b)) => cfg.notify_connection = b,
                    ("notifications", "battery_replaced", Val::Bool(b)) => cfg.notify_battery_replaced = b,
                    ("notifications", "defer_to_powerdevil", Val::Bool(b)) => cfg.defer_to_powerdevil = b,
                    // Sections that belong to another program sharing this file (the
                    // former screen/MQTT bridge, now lg-ddc-control): not ours, so no warning.
                    (s, _, _) if FOREIGN_SECTIONS.contains(&s) => {}
                    (s, k, _) => warn.push(format!("line {}: unknown or mistyped key [{s}] {k}", n + 1)),
                }
            }
            cfg.alerts = AlertConfig::new(
                thresholds.unwrap_or_else(|| cfg.alerts.thresholds().to_vec()),
                hysteresis,
                critical,
            );
            (cfg, warn)
        }
    }

    /// Everything the former reader understood, valid TOML or not.
    const CORPUS: &[&str] = &[
        "",
        "\n\n# only a comment\n",
        "# comment\n[alerts]\nthresholds = [30, 15, 5]  # %\nhysteresis = 3\ncritical = 5\nenabled = true\n\n[notifications]\nconnection = false\nbattery_replaced = true\n",
        "[alerts]\nthresholds=[20,10]\nhysteresis = 4.5\ncritical = 10\n",
        "[alerts]\nthresholds = [\n  40,\n  20, # second\n]\n",
        "\u{feff}[alerts]\nthresholds = [25]\nenabled = false\n",
        "[alerts]\nthresholds = [\n40,\n20]\n",
        "[alerts]\nthresholds = [\n40,\n",
        "[alerts]\nthresholds = []\n",
        "[alerts]\nthresholds = [30, 150]\n",
        "[alerts]\nthresholds = [0, 100, -3]\ncritical = 0\n",
        "[alerts]\nthresholds = [1_0, 5]\nhysteresis = 1_0\ncritical = 9_9\n",
        "[alerts]\nhysteresis = 1e1\n",
        "[alerts]\nhysteresis = -4\ncritical = -1\n",
        "[alerts]\nhysteresis = 250.5\ncritical = 100\n",
        "[alerts]\nhysteresis = true\ncritical = 2.5\nenabled = 1\nthresholds = 30\n",
        "[alerts]\nthresholds = [30, 150, x]\nnope\nhysteresis = \"3\"\n[other]\nfoo = 1\n[alerts]\ncritical = 300\n",
        "[alerts]\nenabled = false\n[alerts]\nenabled = true\n",
        "[alerts]\nenabled = false\nenabled = true\ncritical = 7\ncritical = 8\n",
        "[battery]\nchemistry = \"nimh\"  # Eneloop\n",
        "[battery]\nchemistry = \"Unknown\"\n",
        "[battery]\nchemistry = \"zinc\"\n",
        "[battery]\nchemistry = 3\n",
        "[battery]\nchemistry = \"li#thium\" # a hash inside the string\n",
        "[display]\napple_percent = false\n",
        "[display]\napple_percent = 3\n",
        "[notifications]\ndefer_to_powerdevil = false\nconnection = false\nbattery_replaced = false\n",
        "[notifications]\ndefer_to_powerdevil = 3\n",
        "[apple]\nwill_shutdown = false  # keep the keyboard untouched\n",
        "[apple]\nwill_shutdown = \"no\"\n",
        "[apple]\nwill_shutdown = true\nname_change = true\n",
        "[apple]\ndisconnect_on_breaker = false\n",
        "[apple]\ndisconnect_on_breaker = 0\n",
        "[apple]\nallow_device_name_write = true  # left over from an older version\n",
        "[apple]\nallow_device_name_write = 1\n",
        "[apple]\nallow_device_name_write = \"true\"\n",
        "[apple]\nallow_device_name_write = yes\n",
        "[display]\nallow_device_name_write = true\n",
        "allow_device_name_write = true\n",
        "enabled = true\n[alerts]\nenabled = false\n",
        "[apple]\nwill_shutdwn = true\n",
        "[apple]\ncall_audio_hint = \"auto\"\n",
        "[ddc]\nbus = \"/dev/i2c-3\"\n[mqtt]\nbroker = \"x\"\nport = 1883\n[monitor]\nmodel = \"m\"\n[brightness]\nmin = 2\nlamp_entity = \"l\"\n[alerts]\nenabled = true\n",
        "[mqtt]\nenabled = true\nthresholds = [1, 2]\n[apple]\nwill_shutdown = false\n",
        "[ alerts ]\nenabled = false\n",
        "[unknown]\n",
        "[unknown]\na = 1\nb = 2\n[display]\napple_percent = false\n",
        "[alerts]\r\nenabled = false\r\ncritical = 12\r\n",
        "  [alerts]  \n   enabled   =   false   \n",
        "[alerts]\nenabled =\n",
        "[alerts]\nenabled = false # one\n# two\n\n[battery] # three\nchemistry = \"lithium\"\n",
    ];

    #[test]
    fn same_result_as_the_former_hand_written_reader() {
        // #12: config and warnings, word for word and in the same order.
        for doc in CORPUS {
            assert_eq!(parse(doc), legacy::parse(doc), "{doc:?}");
        }
    }

    #[test]
    fn the_only_wording_that_changed_is_for_a_line_without_a_key() {
        // `= 3` was "unknown or mistyped key [alerts] " (empty key); the crate
        // refuses the statement: still exactly one warning, nothing read.
        let (c, w) = parse("[alerts]\n= 3\n");
        assert_eq!(c, Config::default());
        assert_eq!(w, ["line 2: unreadable value for "]);
    }

    #[test]
    fn a_valid_document_and_its_broken_copy_read_the_same_keys() {
        // The two ways in (whole document / statement by statement) agree:
        // appending a bad line to a valid file costs that line, nothing else.
        for doc in CORPUS {
            if DeTable::parse(doc.trim_start_matches('\u{feff}')).is_err() {
                continue;
            }
            let (c, w) = parse(doc);
            let broken = format!("{doc}\n[akm_test]\nstray word\n");
            let (cb, wb) = parse(&broken);
            assert_eq!(c, cb, "{doc:?}");
            assert_eq!(wb.len(), w.len() + 1, "{doc:?}: {wb:?}");
            assert_eq!(wb[..w.len()], w[..], "{doc:?}");
            assert!(wb[w.len()].ends_with("not a `key = value` line"), "{wb:?}");
        }
    }

    #[test]
    fn real_toml_that_the_former_reader_refused_is_now_understood() {
        // #12: what the crate brings. Each of these raised a warning (or was
        // misread) with the hand-written subset.
        let (c, w) = parse("[battery]\nchemistry = 'nimh'\n");
        assert!(w.is_empty() && c.chemistry == Chemistry::Nimh, "{w:?}");
        let (c, w) = parse("[alerts]\nthresholds = [0x1E, 0o17]\nhysteresis = +2.5\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            (c.alerts.thresholds(), c.alerts.hysteresis),
            (&[30u8, 15][..], 2.5)
        );
        let (c, w) = parse("alerts = { thresholds = [40], enabled = false }\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.alerts.thresholds() == [40] && !c.alerts_enabled);
        let (c, w) = parse("alerts.enabled = false\n\"apple\".\"will_shutdown\" = false\n");
        assert!(
            w.is_empty() && !c.alerts_enabled && !c.will_shutdown,
            "{w:?}"
        );
        // Another program's section may hold anything: sub-tables, arrays of
        // strings, multi-line strings, arrays of tables, dates.
        let (c, w) = parse(
            "[mqtt]\ntopics = [\"a\", \"b\"]\nnote = \"\"\"\n[alerts]\nenabled = false\n\"\"\"\nsince = 2026-10-02\n[mqtt.tls]\nca = 'x'\n[[monitor.outputs]]\nname = \"DP-1\"\n[ddc.\"bus\"]\nn = 3\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            c,
            Config::default(),
            "the [alerts] inside the string is text"
        );
    }

    #[test]
    fn warnings_carry_the_line_of_the_key_in_a_valid_document() {
        let (_, w) = parse("# one\n[display]\n\napple_percent = 3\n[apple]\nwill_shutdown = true\nzzz = 1\naaa = [\n 1,\n]\n\n[alerts]\ncritical = 300\n");
        assert_eq!(
            w,
            [
                "line 4: unknown or mistyped key [display] apple_percent",
                "line 7: unknown or mistyped key [apple] zzz",
                "line 8: unknown or mistyped key [apple] aaa",
                "line 13: critical must be in 0..=99",
            ]
        );
        // Not ours, wherever TOML allows it: a top-level key, a date, an array
        // of tables named as one of our sections, a sub-table of ours.
        let (c, w) = parse("when = 2026-10-02\nenabled = false\n[[alerts]]\nenabled = false\n[apple.extra]\nwill_shutdown = false\n");
        assert_eq!(c, Config::default());
        assert_eq!(
            w,
            [
                "line 1: unknown or mistyped key [] when",
                "line 2: unknown or mistyped key [] enabled",
                "line 3: unknown or mistyped key [] alerts",
                "line 5: unknown or mistyped key [apple] extra",
            ]
        );
    }

    #[test]
    fn values_that_would_poison_the_alerts_are_refused() {
        for bad in ["inf", "-inf", "nan", "\"3\"", "[3]", "true"] {
            let (c, w) = parse(&format!("[alerts]\nhysteresis = {bad}\n"));
            assert_eq!(c.alerts.hysteresis, DEFAULT_HYSTERESIS, "{bad}");
            assert_eq!(w.len(), 1, "{bad}: {w:?}");
        }
        for bad in ["[30, 15.5]", "[30, \"15\"]", "[[30]]", "30", "\"30\""] {
            let (c, w) = parse(&format!("[alerts]\nthresholds = {bad}\n"));
            assert_eq!(c.alerts, AlertConfig::default(), "{bad}");
            assert_eq!(w.len(), 1, "{bad}: {w:?}");
        }
        // i64 overflow is a syntax error for the crate: one warning, default kept.
        let (c, w) = parse("[alerts]\ncritical = 99999999999999999999\nenabled = false\n");
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(c.alerts.critical_at == 5 && !c.alerts_enabled);
    }

    #[test]
    fn a_foreign_section_stays_silent_in_a_file_that_is_not_valid_toml() {
        // The file is broken elsewhere (a stray word in [alerts]): the lines of
        // the other program are still none of our business.
        let (c, w) = parse("[mqtt]\ntopics = [\"a\"]\nbroken line\n[mqtt.tls]\nca = nope\n[alerts]\nstray\nenabled = false\n");
        assert_eq!(w, ["line 7: not a `key = value` line"]);
        assert!(!c.alerts_enabled);
    }

    #[test]
    fn three_spellings_toml_forbids_are_no_longer_taken() {
        // Documented in docs/CONFIGURATION.md: the former reader read these
        // values; TOML forbids them, so each is now one warning and the default.
        for (doc, was) in [
            ("[alerts]\ncritical = 05\n", "critical 5"),
            ("[alerts]\nhysteresis = 4.\n", "hysteresis 4"),
            ("[alerts]\nthresholds = [40,,20]\n", "thresholds 40, 20"),
        ] {
            let (old, old_w) = legacy::parse(doc);
            assert!(old_w.is_empty(), "{doc:?} was read ({was}): {old_w:?}");
            let (c, w) = parse(doc);
            assert_eq!(w.len(), 1, "{doc:?}: {w:?}");
            assert!(w[0].starts_with("line 2: unreadable value for "), "{w:?}");
            assert_eq!(c, Config::default(), "{doc:?}");
            // (critical = 05 gave the default 5 anyway; the two others differ.)
            let _ = old;
        }
    }

    #[test]
    fn missing_file_is_default() {
        let (c, w) = load(Path::new("/nonexistent/akm/config.toml"));
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
        assert!(default_path().ends_with(REL_PATH));
    }
}
