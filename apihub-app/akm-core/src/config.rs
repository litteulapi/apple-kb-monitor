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
//!
//! [display]
//! apple_percent = true       # also show the percentage "as macOS shows it" (#213)
//!
//! [apple]
//! will_shutdown = true       # tell the keyboard once, at shutdown / restart, what macOS tells it (#191)
//! ```
//!
//! Only this small TOML subset is read (sections, integers, floats, booleans,
//! arrays of integers, possibly spread over several lines; an UTF-8 BOM is
//! skipped; `#` inside a quoted string is not a comment). Unknown keys and
//! malformed lines are reported as warnings and otherwise ignored; a missing
//! file means the defaults.

use std::path::{Path, PathBuf};

use crate::alerts::{AlertConfig, DEFAULT_HYSTERESIS};
use crate::chemistry::Chemistry;

const REL_PATH: &str = "apple-kb-monitor/config.toml";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub alerts: AlertConfig,
    pub alerts_enabled: bool,
    pub notify_connection: bool,
    pub notify_battery_replaced: bool,
    /// Declared chemistry of the batteries (#178), default alkaline.
    pub chemistry: Chemistry,
    /// Show the percentage as macOS displays it, labelled "Apple display"
    /// (#213). Pure host-side computation, default on.
    pub apple_percent: bool,
    /// Send `WillShutdown` (Feature `0x40`, the id alone) to the keyboard when
    /// the computer shuts down or restarts, as macOS does at every shutdown
    /// (#191). Default **on**: it is what Apple sends; `false` sends nothing.
    pub will_shutdown: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alerts: AlertConfig::default(),
            alerts_enabled: true,
            notify_connection: true,
            notify_battery_replaced: true,
            chemistry: Chemistry::default(),
            apple_percent: true,
            will_shutdown: true,
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

/// Parse the file content. Returns the config and human-readable warnings.
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
            ("notifications", "connection", Val::Bool(b)) => cfg.notify_connection = b,
            ("notifications", "battery_replaced", Val::Bool(b)) => cfg.notify_battery_replaced = b,
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
        assert_eq!(c.chemistry, Chemistry::Alkaline, "invalid value: default kept");
        assert_eq!(w.len(), 1, "{w:?}");
        let (_, w) = parse("[battery]\nchemistry = 3\n");
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn missing_file_is_default() {
        let (c, w) = load(Path::new("/nonexistent/akm/config.toml"));
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
        assert!(default_path().ends_with(REL_PATH));
    }
}
