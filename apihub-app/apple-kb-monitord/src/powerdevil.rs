//! KDE PowerDevil and the low-battery alerts (#254).
//!
//! PowerDevil follows the keyboard through UPower (the kernel `power_supply`
//! `hid-<mac>-battery`) and raises its own "Keyboard Battery Low" notification
//! (`Event/lowperipheralbattery` of `powerdevil.notifyrc`) at
//! `[BatteryManagement] PeripheralBatteryLowLevel` of `powerdevilrc` (default
//! 10 %). When it does, our 30 / 15 / 5 % alerts are a second voice on the
//! same device. With `[notifications] defer_to_powerdevil = true` (default)
//! they shrink to **one** distinct reminder, worded as an estimate from the
//! batteries, and only when the estimate adds information; the keyboard's own
//! `0x30` alerts are not touched.
//!
//! Nothing here writes any KDE configuration: files are only read.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `PeripheralBatteryLowLevel` when `powerdevilrc` does not set it (amont).
pub const DEFAULT_LOW_LEVEL: u8 = 10;
/// Event of `powerdevil.notifyrc` for peripheral batteries.
pub const EVENT: &str = "lowperipheralbattery";
/// Session bus name owned by PowerDevil.
pub const BUS_NAME: &str = "org.kde.Solid.PowerManagement";
const CACHE: Duration = Duration::from_secs(60);

/// What PowerDevil does about peripheral batteries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Percentage at which it warns.
    pub low_level: u8,
    /// Its `lowperipheralbattery` event still shows a popup.
    pub popup: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self { low_level: DEFAULT_LOW_LEVEL, popup: true }
    }
}

/// Value of `key` in the first of `files` (user file first) that sets it,
/// in any group (the group is `BatteryManagement` in Plasma 6).
fn lookup(files: &[String], key: &str) -> Option<String> {
    files.iter().find_map(|f| {
        f.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    })
}

/// Value of `key` inside `[group]`.
fn lookup_in(files: &[String], group: &str, key: &str) -> Option<String> {
    let head = format!("[{group}]");
    files.iter().find_map(|f| {
        let mut inside = false;
        for l in f.lines() {
            let l = l.trim();
            if l.starts_with('[') {
                inside = l == head;
            } else if inside {
                if let Some((k, v)) = l.split_once('=') {
                    if k.trim() == key {
                        return Some(v.trim().to_string());
                    }
                }
            }
        }
        None
    })
}

/// Policy from the contents of `powerdevilrc` and `powerdevil.notifyrc`
/// (user file first, then the system ones).
pub fn parse_policy(powerdevilrc: &[String], notifyrc: &[String]) -> Policy {
    let low_level = lookup(powerdevilrc, "PeripheralBatteryLowLevel")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v <= 100)
        .unwrap_or(DEFAULT_LOW_LEVEL);
    // `Action=` lists the enabled ways (Popup, Sound, Taskbar, Logfile...);
    // an event without the key keeps the shipped default (a popup).
    let group = format!("Event/{EVENT}");
    let popup = lookup_in(notifyrc, &group, "Action").is_none_or(|a| a.split('|').any(|x| x.trim() == "Popup"));
    Policy { low_level, popup }
}

/// Does PowerDevil raise a notification for this keyboard?
pub trait Probe: Send + Sync + std::fmt::Debug {
    fn covers(&self) -> bool;
    /// Threshold of PowerDevil, for the wording of our reminder.
    fn low_level(&self) -> u8 {
        DEFAULT_LOW_LEVEL
    }
}

/// What to do with one of our percentage crossings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Send it as usual.
    Normal,
    /// Send the single distinct reminder instead.
    Reminder,
    /// Say nothing: PowerDevil speaks.
    Skip,
}

/// `estimate`: the alert is computed on the chemistry estimate, not on the
/// keyboard's own percentage (the one PowerDevil sees); `reminded`: the
/// reminder was already sent for this set of batteries.
pub fn plan(defer: bool, covered: bool, estimate: bool, reminded: bool) -> Plan {
    match (defer && covered, estimate, reminded) {
        (false, _, _) => Plan::Normal,
        (true, true, false) => Plan::Reminder,
        (true, _, _) => Plan::Skip,
    }
}

/// Sources of the real probe, replaceable in tests.
#[derive(Debug, Clone)]
pub struct Sources {
    pub config_dirs: Vec<PathBuf>,
    pub power_supply: PathBuf,
}

impl Default for Sources {
    fn default() -> Self {
        let home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        Self {
            config_dirs: home.into_iter().chain([PathBuf::from("/etc/xdg")]).collect(),
            power_supply: PathBuf::from("/sys/class/power_supply"),
        }
    }
}

fn read_all(dirs: &[PathBuf], name: &str) -> Vec<String> {
    dirs.iter().filter_map(|d| std::fs::read_to_string(d.join(name)).ok()).collect()
}

/// The kernel exposes the keyboard as `hid-<mac>-battery...`: that is the
/// device UPower (hence PowerDevil) lists.
fn kernel_battery_present(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.starts_with("hid-") && n.contains("battery")
        })
    })
}

fn powerdevil_on_bus() -> bool {
    zbus::blocking::Connection::session()
        .and_then(|c| zbus::blocking::fdo::DBusProxy::new(&c).and_then(|p| {
            p.name_has_owner(BUS_NAME.try_into().map_err(zbus::Error::from)?).map_err(Into::into)
        }))
        .unwrap_or(false)
}

/// Real probe: PowerDevil running, popup enabled, keyboard in UPower's view.
/// Re-evaluated at most once a minute (settings can change at any time).
#[derive(Debug, Default)]
pub struct SystemProbe {
    sources: Sources,
    cache: Mutex<Option<(Instant, bool, u8)>>,
}

impl SystemProbe {
    pub fn with_sources(sources: Sources) -> Self {
        Self { sources, cache: Mutex::new(None) }
    }

    fn evaluate(&self, on_bus: bool) -> (bool, u8) {
        let pol = parse_policy(
            &read_all(&self.sources.config_dirs, "powerdevilrc"),
            &read_all(&self.sources.config_dirs, "powerdevil.notifyrc"),
        );
        let covers = on_bus && pol.popup && kernel_battery_present(&self.sources.power_supply);
        (covers, pol.low_level)
    }

    fn state(&self) -> (bool, u8) {
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, v, l)) = *c {
            if t.elapsed() < CACHE {
                return (v, l);
            }
        }
        let (v, l) = self.evaluate(powerdevil_on_bus());
        tracing::info!("PowerDevil alerts this keyboard: {v} (threshold {l} %)");
        *c = Some((Instant::now(), v, l));
        (v, l)
    }
}

impl Probe for SystemProbe {
    fn covers(&self) -> bool {
        self.state().0
    }
    fn low_level(&self) -> u8 {
        self.state().1
    }
}

/// Test probe with a fixed answer.
#[derive(Debug)]
pub struct Fixed(pub bool);

impl Probe for Fixed {
    fn covers(&self) -> bool {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Vec<String> {
        vec![x.to_string()]
    }

    #[test]
    fn defaults_when_nothing_is_set() {
        assert_eq!(parse_policy(&[], &[]), Policy { low_level: 10, popup: true });
        assert_eq!(parse_policy(&s("[General]\nX=1\n"), &s("")), Policy::default());
    }

    #[test]
    fn user_value_wins_over_system_one() {
        let user = "[BatteryManagement]\nPeripheralBatteryLowLevel=20\n".to_string();
        let sys = "[BatteryManagement]\nPeripheralBatteryLowLevel=7\n".to_string();
        assert_eq!(parse_policy(&[user, sys], &[]).low_level, 20);
        assert_eq!(parse_policy(&s("PeripheralBatteryLowLevel=400\n"), &[]).low_level, 10);
    }

    #[test]
    fn disabled_popup_means_powerdevil_stays_silent() {
        let off = "[Event/lowperipheralbattery]\nAction=\n[Event/other]\nAction=Popup\n";
        assert!(!parse_policy(&[], &s(off)).popup);
        let on = "[Event/lowperipheralbattery]\nAction=Popup|Sound\n";
        assert!(parse_policy(&[], &s(on)).popup);
        let other = "[Event/other]\nAction=\n";
        assert!(parse_policy(&[], &s(other)).popup, "another event does not count");
    }

    #[test]
    fn plan_matrix() {
        assert_eq!(plan(false, true, true, false), Plan::Normal);
        assert_eq!(plan(true, false, true, false), Plan::Normal);
        assert_eq!(plan(true, true, true, false), Plan::Reminder);
        assert_eq!(plan(true, true, true, true), Plan::Skip);
        assert_eq!(plan(true, true, false, false), Plan::Skip, "firmware % is what PowerDevil sees");
    }

    #[test]
    fn probe_needs_the_bus_name_the_popup_and_a_kernel_battery() {
        let dir = std::env::temp_dir().join(format!("akm-pd-{}", std::process::id()));
        let cfg = dir.join("cfg");
        let ps = dir.join("ps");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(&ps).unwrap();
        let p = SystemProbe::with_sources(Sources { config_dirs: vec![cfg.clone()], power_supply: ps.clone() });
        assert_eq!(p.evaluate(true), (false, 10), "no kernel battery");
        std::fs::create_dir_all(ps.join("hid-aa:bb:cc:dd:ee:f1-battery-71")).unwrap();
        assert_eq!(p.evaluate(true), (true, 10));
        assert_eq!(p.evaluate(false), (false, 10), "PowerDevil not running");
        std::fs::write(cfg.join("powerdevil.notifyrc"), "[Event/lowperipheralbattery]\nAction=\n").unwrap();
        assert!(!p.evaluate(true).0, "popup disabled by the user");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
