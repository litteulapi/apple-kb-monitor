//! `akmctl doctor`: one-command diagnosis of the keyboard's Bluetooth link
//! (#147, docs/RECONNEXION-PAIRAGE.md). Read-only: it never writes anything,
//! never pages the keyboard, never sends a HID request.
//!
//! Pure checks (parsers, verdict) are unit-tested; gathering reads BlueZ on
//! the system bus, sysfs, `/etc`, the journal and the daemon's `Link` object.
//! As root it also compares the stored link key with the kernel's (by
//! fingerprint only, the key itself is never printed).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use akm_core::model::is_keyboard_device;
use akm_core::recovery::{classify_journal, JournalKind};
use serde_json::{json, Value};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

pub const UDEV_RULE: &str = "/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules";
pub const MAIN_CONF: &str = "/etc/bluetooth/main.conf";
pub const UPOWER_CONF: &str = "/etc/UPower/UPower.conf";
const LINK_PATH: &str = "/com/agenceapi/AppleKbMonitor1/Link";
const LINK_IFACE: &str = "com.agenceapi.AppleKbMonitor1.Link";
const BUS_NAME: &str = "com.agenceapi.AppleKbMonitor1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Bad,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Ok => "[ ok ]",
            Level::Info => "[info]",
            Level::Warn => "[ !! ]",
            Level::Bad => "[ KO ]",
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Bad => "bad",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub topic: &'static str,
    pub text: String,
    /// What to do about it, if anything.
    pub fix: Option<String>,
}

fn f(level: Level, topic: &'static str, text: impl Into<String>, fix: Option<&str>) -> Finding {
    Finding {
        level,
        topic,
        text: text.into(),
        fix: fix.map(str::to_string),
    }
}

// ── pure checks ────────────────────────────────────────────────────────────

/// Active `key = value` entries of an INI-like file, with their section.
pub fn ini_entries(text: &str) -> Vec<(String, String, String)> {
    let mut sec = String::new();
    let mut out = Vec::new();
    for raw in text.lines() {
        let l = raw.trim();
        if l.starts_with('#') || l.starts_with(';') || l.is_empty() {
            continue;
        }
        if let Some(s) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            sec = s.trim().to_string();
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            out.push((sec.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// BlueZ `main.conf`: reconnection keys in the right section, FastConnectable.
pub fn check_main_conf(text: &str) -> Vec<Finding> {
    let e = ini_entries(text);
    let mut out = Vec::new();
    let misplaced: Vec<_> = e
        .iter()
        .filter(|(s, k, _)| k.starts_with("Reconnect") && s != "Policy")
        .map(|(s, k, _)| format!("{k} in [{s}]"))
        .collect();
    if !misplaced.is_empty() {
        out.push(f(
            Level::Bad,
            "bluez-conf",
            format!("ignored by bluetoothd: {}", misplaced.join(", ")),
            Some("sudo bluetooth/akm-conf.py bluez --apply && sudo systemctl restart bluetooth"),
        ));
    } else if e
        .iter()
        .any(|(s, k, _)| s == "Policy" && k == "ReconnectAttempts")
    {
        out.push(f(
            Level::Ok,
            "bluez-conf",
            "[Policy] Reconnect* in place",
            None,
        ));
    }
    let fast = e
        .iter()
        .find(|(s, k, _)| s == "General" && k == "FastConnectable")
        .is_some_and(|(_, _, v)| v.eq_ignore_ascii_case("true"));
    if fast {
        out.push(f(Level::Ok, "bluez-conf", "FastConnectable = true", None));
    } else {
        out.push(f(
            Level::Warn,
            "bluez-conf",
            "FastConnectable off: slower answer to the keyboard's page after a key press",
            Some("sudo bluetooth/akm-conf.py bluez --apply && sudo systemctl restart bluetooth"),
        ));
    }
    out
}

/// UPower polls batteries every 30 s unless NoPollBatteries=true (#146).
pub fn check_upower(text: &str) -> Finding {
    let no_poll = ini_entries(text)
        .iter()
        .any(|(_, k, v)| k == "NoPollBatteries" && v.eq_ignore_ascii_case("true"));
    if no_poll {
        f(
            Level::Ok,
            "upower",
            "NoPollBatteries = true (no 30 s HID polling)",
            None,
        )
    } else {
        f(
            Level::Info,
            "upower",
            "UPower reads the keyboard battery every 30 s (2 GET_REPORT over the air): the keyboard never sleeps",
            Some("optional: sudo bluetooth/akm-conf.py upower --apply && sudo systemctl restart upower"),
        )
    }
}

/// USB runtime PM of the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbPower {
    pub id: String,
    pub control: String,
    pub runtime_status: String,
    pub suspended_ms: u64,
    pub delay_ms: String,
}

pub fn check_usb_power(p: Option<&UsbPower>, rule_installed: bool) -> Finding {
    let Some(p) = p else {
        return f(
            Level::Info,
            "adapter-pm",
            "adapter is not on USB (or not found)",
            None,
        );
    };
    if p.control == "on" {
        f(
            Level::Ok,
            "adapter-pm",
            format!("{} autosuspend off (power/control=on)", p.id),
            None,
        )
    } else {
        f(
            Level::Warn,
            "adapter-pm",
            format!(
                "{} USB autosuspend active (control={}, delay {} ms, now {}, {} s suspended since boot){}",
                p.id,
                p.control,
                p.delay_ms,
                p.runtime_status,
                p.suspended_ms / 1000,
                if rule_installed { "; rule installed but not applied yet" } else { "" }
            ),
            Some(
                "sudo install -m644 udev/61-akm-bt-adapter-no-autosuspend.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger --action=add --subsystem-match=usb --attr-match=idVendor=8087",
            ),
        )
    }
}

/// Counts of relevant bluetoothd journal lines (current boot).
pub fn summarize_journal(lines: &[String], mac: Option<&str>) -> Vec<(JournalKind, usize, String)> {
    let mut map: HashMap<JournalKind, (usize, String)> = HashMap::new();
    for l in lines {
        let about_other_device = mac.is_some_and(|m| {
            l.contains(':')
                && contains_mac(l)
                && !l.to_ascii_uppercase().contains(&m.to_ascii_uppercase())
        });
        if about_other_device {
            continue;
        }
        if let Some(k) = classify_journal(l) {
            let e = map.entry(k).or_insert((0, String::new()));
            e.0 += 1;
            e.1 = l.split_whitespace().next().unwrap_or("").to_string();
        }
    }
    let mut v: Vec<_> = map.into_iter().map(|(k, (n, t))| (k, n, t)).collect();
    v.sort_by_key(|(k, _, _)| k.as_str());
    v
}

fn contains_mac(l: &str) -> bool {
    l.as_bytes().windows(17).any(|w| {
        w.iter().enumerate().all(|(i, c)| {
            if i % 3 == 2 {
                *c == b':' || *c == b'_'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    })
}

pub fn journal_findings(s: &[(JournalKind, usize, String)]) -> Vec<Finding> {
    let mut out = Vec::new();
    for (k, n, last) in s {
        let (lvl, text, fix) = match k {
            JournalKind::PageTimeout => (
                Level::Info,
                format!("{n} page timeout(s) (keyboard asleep or not listening), last {last}"),
                None,
            ),
            JournalKind::GetReportTimeout => (
                Level::Warn,
                format!("{n} HIDP GET_REPORT timeout(s), last {last}"),
                Some("avoid HID scanning tools while measuring; see docs/RECONNEXION-PAIRAGE.md §3.5"),
            ),
            JournalKind::Auth => (
                Level::Bad,
                format!("{n} authentication / key error(s), last {last}"),
                Some("akmctl repair"),
            ),
            JournalKind::Refused => (
                Level::Info,
                format!("{n} connection(s) reset/refused by the keyboard, last {last}"),
                None,
            ),
            JournalKind::ConfigIgnored => (
                Level::Bad,
                format!("{n} main.conf key(s) ignored by bluetoothd at start, last {last}"),
                Some("sudo bluetooth/akm-conf.py bluez --apply && sudo systemctl restart bluetooth"),
            ),
            JournalKind::StorageError => (
                Level::Bad,
                format!("{n} BlueZ storage write failure(s) (a new link key may be lost at reboot), last {last}"),
                Some("free disk space (btrfs: check metadata), then verify with sudo akmctl doctor"),
            ),
        };
        out.push(Finding {
            level: lvl,
            topic: "journal",
            text,
            fix: fix.map(str::to_string),
        });
    }
    out
}

/// BlueZ facts about one keyboard.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KbFacts {
    pub mac: String,
    pub name: String,
    pub path: String,
    pub paired: bool,
    pub bonded: bool,
    pub trusted: bool,
    pub connected: bool,
    pub blocked: bool,
    pub legacy: bool,
    pub wake_allowed: bool,
}

pub fn keyboard_findings(k: Option<&KbFacts>, adapter_powered: Option<bool>) -> Vec<Finding> {
    let mut out = Vec::new();
    match adapter_powered {
        Some(true) => out.push(f(Level::Ok, "adapter", "Bluetooth adapter powered", None)),
        Some(false) => out.push(f(
            Level::Bad,
            "adapter",
            "Bluetooth adapter off",
            Some("bluetoothctl power on"),
        )),
        None => out.push(f(
            Level::Bad,
            "adapter",
            "BlueZ unreachable (bluetooth.service stopped?)",
            Some("systemctl status bluetooth"),
        )),
    }
    let Some(k) = k else {
        out.push(f(
            Level::Bad,
            "pairing",
            "no paired Apple keyboard known to BlueZ",
            Some("akmctl repair (pairing assistant)"),
        ));
        return out;
    };
    if !(k.paired || k.bonded) {
        out.push(f(
            Level::Bad,
            "pairing",
            format!("{} is not paired", k.mac),
            Some("akmctl repair"),
        ));
    } else {
        out.push(f(
            Level::Ok,
            "pairing",
            format!(
                "{} \u{201c}{}\u{201d} paired{}{}",
                k.mac,
                k.name,
                if k.bonded { ", bonded" } else { "" },
                if k.legacy { ", legacy PIN pairing" } else { "" }
            ),
            None,
        ));
    }
    if !k.trusted {
        out.push(f(
            Level::Warn,
            "pairing",
            "not trusted: BlueZ may refuse the keyboard's own reconnection",
            Some(&format!("bluetoothctl trust {}", k.mac)),
        ));
    }
    if k.blocked {
        out.push(f(
            Level::Bad,
            "pairing",
            "blocked in BlueZ",
            Some(&format!("bluetoothctl unblock {}", k.mac)),
        ));
    }
    out.push(if k.connected {
        f(Level::Ok, "link", "connected", None)
    } else {
        f(
            Level::Info,
            "link",
            "not connected (asleep? press a key)",
            None,
        )
    });
    out
}

/// Stored vs kernel link key, by fingerprint (root only).
pub fn key_finding(stored: Option<&str>, kernel: Option<&str>, root: bool) -> Finding {
    if !root {
        return f(
            Level::Info,
            "link-key",
            "link key not checked (needs root: sudo akmctl doctor)",
            None,
        );
    }
    match (stored, kernel) {
        (None, _) => f(
            Level::Bad,
            "link-key",
            "no [LinkKey] stored for the keyboard",
            Some("akmctl repair"),
        ),
        (Some(s), Some(k)) if s == k => f(
            Level::Ok,
            "link-key",
            format!("stored key == kernel key (fingerprint {s})"),
            None,
        ),
        (Some(s), Some(k)) => f(
            Level::Bad,
            "link-key",
            format!("stored key {s} != kernel key {k}: will change at next restart"),
            Some("check disk space, then akmctl repair if the keyboard is refused"),
        ),
        (Some(s), None) => f(
            Level::Warn,
            "link-key",
            format!("stored key {s}; kernel copy not readable (debugfs not mounted?)"),
            None,
        ),
    }
}

/// Short, non-reversible fingerprint of a hex key.
pub fn fingerprint(hex: &str) -> String {
    // FNV-1a 64 — enough to compare two copies, useless to recover the key.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in hex
        .trim()
        .trim_start_matches("0x")
        .to_ascii_lowercase()
        .bytes()
    {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:012x}", h >> 16)
}

/// Overall verdict: (level, one-line advice).
pub fn verdict(findings: &[Finding], health: Option<&str>, connected: bool) -> (Level, String) {
    let worst = findings.iter().map(|x| x.level).max().unwrap_or(Level::Ok);
    if health == Some("auth-failed")
        || findings
            .iter()
            .any(|x| x.topic == "link-key" && x.level == Level::Bad)
    {
        return (
            Level::Bad,
            "the pairing is refused or missing: run `akmctl repair`".into(),
        );
    }
    if findings
        .iter()
        .any(|x| x.topic == "pairing" && x.level == Level::Bad)
    {
        return (Level::Bad, "no usable pairing: run `akmctl repair`".into());
    }
    if connected {
        let msg = if worst >= Level::Warn {
            "link up; apply the fixes above to keep it reliable"
        } else {
            "link up and configuration sound"
        };
        return (worst.max(Level::Ok), msg.into());
    }
    (
        worst.max(Level::Info),
        "keyboard not connected: press a key and wait 10 s; the daemon pages it on its own. \
Re-pair only if `akmctl doctor` reports auth-failed"
            .into(),
    )
}

// ── gathering ──────────────────────────────────────────────────────────────

type Props = HashMap<String, OwnedValue>;

fn pb(p: &Props, k: &str) -> bool {
    p.get(k)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}
fn ps(p: &Props, k: &str) -> Option<String> {
    p.get(k)
        .and_then(|v| <&str>::try_from(v).ok().map(str::to_string))
}

/// Paired Apple keyboards and the adapter's Powered flag.
pub fn bluez_facts(conn: &Connection) -> zbus::Result<(Vec<KbFacts>, Option<bool>)> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
        .destination("org.bluez")?
        .path("/")?
        .build()?;
    let mut kbs = Vec::new();
    let mut powered = None;
    for (path, ifaces) in om.get_managed_objects()? {
        for (iface, p) in &ifaces {
            match iface.as_str() {
                "org.bluez.Adapter1" => powered = Some(pb(p, "Powered")),
                "org.bluez.Device1" => {
                    let class = p.get("Class").and_then(|v| u32::try_from(v).ok());
                    if !ps(p, "Modalias").is_some_and(|m| is_keyboard_device(&m, class)) {
                        continue;
                    }
                    let mac = ps(p, "Address").unwrap_or_default().to_ascii_uppercase();
                    kbs.push(KbFacts {
                        name: ps(p, "Alias")
                            .or_else(|| ps(p, "Name"))
                            .unwrap_or_else(|| mac.clone()),
                        mac,
                        path: path.to_string(),
                        paired: pb(p, "Paired"),
                        bonded: pb(p, "Bonded"),
                        trusted: pb(p, "Trusted"),
                        connected: pb(p, "Connected"),
                        blocked: pb(p, "Blocked"),
                        legacy: pb(p, "LegacyPairing"),
                        wake_allowed: pb(p, "WakeAllowed"),
                    });
                }
                _ => {}
            }
        }
    }
    // paired first, then connected
    kbs.sort_by_key(|k| (!(k.paired || k.bonded), !k.connected));
    Ok((kbs, powered))
}

/// USB device directory of `hciN` and its power state.
pub fn usb_power(sys: &Path, hci: &str) -> Option<UsbPower> {
    let dev = std::fs::canonicalize(sys.join("class/bluetooth").join(hci).join("device")).ok()?;
    // .../1-14/1-14:1.0 -> the interface; the device is its parent.
    let usb = if dev.join("idVendor").exists() {
        dev
    } else {
        dev.parent()?.to_path_buf()
    };
    let rd = |n: &str| {
        std::fs::read_to_string(usb.join(n))
            .ok()
            .map(|s| s.trim().to_string())
    };
    Some(UsbPower {
        id: format!("{}:{}", rd("idVendor")?, rd("idProduct")?),
        control: rd("power/control").unwrap_or_default(),
        runtime_status: rd("power/runtime_status").unwrap_or_default(),
        suspended_ms: rd("power/runtime_suspended_time")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        delay_ms: rd("power/autosuspend_delay_ms").unwrap_or_default(),
    })
}

/// hidraw node of the keyboard (by HID_UNIQ) and whether we can read it.
pub fn hidraw_of(sys: &Path, mac: &str) -> Option<(PathBuf, bool)> {
    let want = mac.to_ascii_lowercase();
    for e in std::fs::read_dir(sys.join("class/hidraw")).ok()?.flatten() {
        let ue = std::fs::read_to_string(e.path().join("device/uevent")).unwrap_or_default();
        if ue.lines().any(|l| {
            l.strip_prefix("HID_UNIQ=")
                .is_some_and(|u| u.eq_ignore_ascii_case(&want))
        }) {
            let dev = Path::new("/dev").join(e.file_name());
            let ok = std::fs::File::open(&dev).is_ok();
            return Some((dev, ok));
        }
    }
    None
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// (stored, kernel) key fingerprints (root only).
fn key_fingerprints(mac: &str) -> (Option<String>, Option<String>) {
    let mut stored = None;
    if let Ok(rd) = std::fs::read_dir("/var/lib/bluetooth") {
        for a in rd.flatten() {
            let info = a.path().join(mac).join("info");
            if let Ok(t) = std::fs::read_to_string(&info) {
                stored = ini_entries(&t)
                    .into_iter()
                    .find(|(s, k, _)| s == "LinkKey" && k == "Key")
                    .map(|(_, _, v)| fingerprint(&v));
            }
        }
    }
    let kernel = std::fs::read_to_string("/sys/kernel/debug/bluetooth/hci0/link_keys")
        .ok()
        .and_then(|t| {
            t.lines()
                .map(|l| l.split_whitespace().collect::<Vec<_>>())
                .find(|w| w.len() >= 3 && w[0].eq_ignore_ascii_case(mac))
                .map(|w| fingerprint(w[2]))
        });
    (stored, kernel)
}

fn journal_lines() -> Option<Vec<String>> {
    let out = Command::new("journalctl")
        .args([
            "-u",
            "bluetooth",
            // A recent window, not the whole boot: an old incident must not keep the
            // verdict (and the self-check) orange until the next reboot.
            "--since",
            "-6 hours",
            "--no-pager",
            "-o",
            "short-iso",
            "-n",
            "5000",
            "-q",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

/// The daemon's link quality (#105, `quality` of a `Link.Status()` entry):
/// disconnections of the last hour / day / 7 days and the relative signal.
/// A warning while the link is unstable (more than 3 unexpected
/// disconnections within an hour). `None` when the daemon keeps none.
pub fn link_quality_finding(entry: &Value) -> Option<Finding> {
    let q = entry.get("quality").filter(|q| q.is_object())?;
    let n = |k: &str| q[k].as_u64().unwrap_or(0);
    let unstable = q["unstable"].as_bool().unwrap_or(false);
    let mut text = format!(
        "link quality: {} disconnection(s) in the last hour, {} in 24 h, {} in 7 days",
        n("disconnects_last_hour"),
        n("disconnects_last_day"),
        n("disconnects_7d")
    );
    if let Some(s) = q.get("signal_7d").filter(|s| s.is_object()) {
        text += &format!(
            "; relative signal over 7 days: mean {} dB, min {} dB ({} measurement(s))",
            s["mean"].as_f64().unwrap_or(0.0),
            s["min"].as_i64().unwrap_or(0),
            s["samples"].as_u64().unwrap_or(0)
        );
    }
    if unstable {
        text += &format!(
            " \u{2014} UNSTABLE: {} unexpected within the hour",
            n("unexpected_last_hour")
        );
    }
    Some(f(
        if unstable { Level::Warn } else { Level::Ok },
        "link-quality",
        text,
        unstable.then_some("check the batteries, the distance, USB 3 devices near the adapter"),
    ))
}

fn daemon_link_status() -> Option<Value> {
    let conn = Connection::session().ok()?;
    let reply = conn
        .call_method(Some(BUS_NAME), LINK_PATH, Some(LINK_IFACE), "Status", &())
        .ok()?;
    let s: String = reply.body().deserialize().ok()?;
    serde_json::from_str(&s).ok()
}

pub struct Report {
    pub findings: Vec<Finding>,
    pub keyboard: Option<KbFacts>,
    pub health: Option<String>,
    pub verdict: (Level, String),
    pub daemon: Option<Value>,
}

/// Gather everything (read-only).
pub fn gather(mac: Option<&str>) -> Report {
    let mut fs = Vec::new();
    let (kbs, powered) = Connection::system()
        .and_then(|c| bluez_facts(&c))
        .unwrap_or_default();
    let kb = match mac {
        Some(m) => kbs.iter().find(|k| k.mac.eq_ignore_ascii_case(m)).cloned(),
        None => kbs.first().cloned(),
    };
    fs.extend(keyboard_findings(kb.as_ref(), powered));
    let root = is_root();
    if let Some(k) = kb.as_ref() {
        let (s, kn) = if root {
            key_fingerprints(&k.mac)
        } else {
            (None, None)
        };
        fs.push(key_finding(s.as_deref(), kn.as_deref(), root));
        match hidraw_of(Path::new("/sys"), &k.mac) {
            Some((dev, true)) => fs.push(f(
                Level::Ok,
                "hidraw",
                format!("{} readable", dev.display()),
                None,
            )),
            Some((dev, false)) => fs.push(f(
                Level::Warn,
                "hidraw",
                format!("{} not readable by this user", dev.display()),
                Some("install udev/70-apple-kb-hidraw.rules (uaccess)"),
            )),
            None if k.connected => fs.push(f(
                Level::Warn,
                "hidraw",
                "connected but no hidraw node (uhid not created yet?)",
                None,
            )),
            None => {}
        }
    }
    let rule = Path::new(UDEV_RULE).exists();
    fs.push(check_usb_power(
        usb_power(Path::new("/sys"), "hci0").as_ref(),
        rule,
    ));
    if let Ok(v) = std::fs::read_to_string("/sys/module/btusb/parameters/enable_autosuspend") {
        fs.push(f(
            Level::Info,
            "adapter-pm",
            format!("btusb enable_autosuspend = {}", v.trim()),
            None,
        ));
    }
    match std::fs::read_to_string(MAIN_CONF) {
        Ok(t) => fs.extend(check_main_conf(&t)),
        Err(e) => fs.push(f(
            Level::Info,
            "bluez-conf",
            format!("{MAIN_CONF}: {e}"),
            None,
        )),
    }
    if let Ok(t) = std::fs::read_to_string(UPOWER_CONF) {
        fs.push(check_upower(&t));
    }
    match journal_lines() {
        Some(lines) => {
            let s = summarize_journal(&lines, kb.as_ref().map(|k| k.mac.as_str()));
            if s.is_empty() {
                fs.push(f(
                    Level::Ok,
                    "journal",
                    "no bluetoothd error this boot",
                    None,
                ));
            }
            fs.extend(journal_findings(&s));
        }
        None => fs.push(f(
            Level::Info,
            "journal",
            "bluetoothd journal not readable (add yourself to group systemd-journal, or use sudo)",
            None,
        )),
    }
    let daemon = daemon_link_status();
    let health = daemon.as_ref().and_then(|v| {
        let arr = v.as_array()?;
        let e = match kb.as_ref() {
            Some(k) => arr
                .iter()
                .find(|e| e["mac"].as_str() == Some(k.mac.as_str())),
            None => arr.first(),
        }?;
        e["health"].as_str().map(str::to_string)
    });
    match (&daemon, &health) {
        (None, _) => fs.push(f(
            Level::Warn,
            "daemon",
            "apple-kb-monitord link keeper not reachable (no automatic reconnection)",
            Some("systemctl --user restart apple-kb-monitord"),
        )),
        (Some(_), Some(h)) => fs.push(f(
            match h.as_str() {
                "connected" | "dormant" | "suspended" => Level::Ok,
                "unreachable" => Level::Warn,
                _ => Level::Bad,
            },
            "daemon",
            format!("link health: {h}"),
            (h == "auth-failed").then_some("akmctl repair"),
        )),
        (Some(_), None) => fs.push(f(
            Level::Info,
            "daemon",
            "keeper running, keyboard not tracked",
            None,
        )),
    }
    // Link quality kept by the daemon (#105).
    let entry = daemon.as_ref().and_then(|v| {
        let arr = v.as_array()?;
        match kb.as_ref() {
            Some(k) => arr
                .iter()
                .find(|e| e["mac"].as_str() == Some(k.mac.as_str())),
            None => arr.first(),
        }
    });
    fs.extend(entry.and_then(link_quality_finding));
    let connected = kb.as_ref().is_some_and(|k| k.connected);
    let verdict = verdict(&fs, health.as_deref(), connected);
    Report {
        findings: fs,
        keyboard: kb,
        health,
        verdict,
        daemon,
    }
}

pub fn to_text(r: &Report) -> String {
    let mut s = String::new();
    for x in &r.findings {
        s.push_str(&format!("{} {:<11} {}\n", x.level.tag(), x.topic, x.text));
        if let Some(fix) = &x.fix {
            s.push_str(&format!("{:>20}\u{2192} {fix}\n", ""));
        }
    }
    s.push_str(&format!(
        "\nverdict: {} {}\n",
        r.verdict.0.tag(),
        r.verdict.1
    ));
    s
}

pub fn to_json(r: &Report) -> Value {
    json!({
        "verdict": {"level": r.verdict.0.as_str(), "advice": r.verdict.1},
        "health": r.health,
        "keyboard": r.keyboard.as_ref().map(|k| json!({
            "mac": k.mac, "name": k.name, "paired": k.paired, "bonded": k.bonded,
            "trusted": k.trusted, "connected": k.connected, "legacy_pairing": k.legacy,
            "wake_allowed": k.wake_allowed,
        })),
        "findings": r.findings.iter().map(|x| json!({
            "level": x.level.as_str(), "topic": x.topic, "text": x.text, "fix": x.fix,
        })).collect::<Vec<_>>(),
        "daemon": r.daemon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #105: the link quality kept by the daemon is one finding of doctor.
    #[test]
    fn link_quality_of_the_daemon_is_a_finding() {
        let stable = serde_json::json!({"mac": "AA:BB:CC:DD:EE:F1", "quality": {
            "disconnects_last_hour": 1, "disconnects_last_day": 2, "disconnects_7d": 9,
            "unexpected_last_hour": 1, "unstable": false,
            "signal_7d": {"samples": 40, "mean": -2.5, "min": -9, "max": 0}
        }});
        let fi = link_quality_finding(&stable).unwrap();
        assert_eq!(fi.level, Level::Ok);
        assert_eq!(fi.topic, "link-quality");
        assert!(
            fi.text
                .contains("1 disconnection(s) in the last hour, 2 in 24 h, 9 in 7 days"),
            "{}",
            fi.text
        );
        assert!(
            fi.text
                .contains("mean -2.5 dB, min -9 dB (40 measurement(s))"),
            "{}",
            fi.text
        );
        assert!(fi.fix.is_none());
        let unstable = serde_json::json!({"quality": {
            "disconnects_last_hour": 5, "disconnects_last_day": 5, "disconnects_7d": 5,
            "unexpected_last_hour": 5, "unstable": true, "signal_7d": null
        }});
        let fi = link_quality_finding(&unstable).unwrap();
        assert_eq!(fi.level, Level::Warn);
        assert!(fi.text.contains("UNSTABLE: 5 unexpected") && !fi.text.contains("signal"));
        assert!(fi.fix.is_some());
        // An older daemon (no `quality`), or none recorded: no finding.
        assert!(link_quality_finding(&serde_json::json!({"mac": "x"})).is_none());
        assert!(link_quality_finding(&serde_json::json!({"quality": null})).is_none());
    }

    const BAD_CONF: &str = "[General]\nExperimental = true\n#FastConnectable = false\n\n[Policy]\n#ReconnectAttempts=7\n\n[AdvMon]\nReconnectUUIDs=00001124-0000-1000-8000-00805f9b34fb\nReconnectAttempts=7\n";
    const GOOD_CONF: &str = "[General]\nFastConnectable = true\n[Policy]\nReconnectAttempts=7\nReconnectIntervals=1,2,4\n";

    #[test]
    fn main_conf_detects_the_advmon_mistake() {
        let v = check_main_conf(BAD_CONF);
        assert!(v
            .iter()
            .any(|x| x.level == Level::Bad && x.text.contains("ReconnectAttempts in [AdvMon]")));
        assert!(v
            .iter()
            .any(|x| x.level == Level::Warn && x.text.contains("FastConnectable")));
        let v = check_main_conf(GOOD_CONF);
        assert!(v.iter().all(|x| x.level == Level::Ok), "{v:?}");
    }

    #[test]
    fn upower_polling() {
        assert_eq!(
            check_upower("[UPower]\nNoPollBatteries=false\n").level,
            Level::Info
        );
        assert_eq!(
            check_upower("[UPower]\nNoPollBatteries = true\n").level,
            Level::Ok
        );
    }

    #[test]
    fn usb_power_findings() {
        let p = UsbPower {
            id: "8087:0026".into(),
            control: "auto".into(),
            runtime_status: "suspended".into(),
            suspended_ms: 430_343,
            delay_ms: "2000".into(),
        };
        let x = check_usb_power(Some(&p), false);
        assert_eq!(x.level, Level::Warn);
        assert!(x.text.contains("430 s") && x.fix.unwrap().contains("61-akm"));
        let on = UsbPower {
            control: "on".into(),
            ..p
        };
        assert_eq!(check_usb_power(Some(&on), true).level, Level::Ok);
        assert_eq!(check_usb_power(None, false).level, Level::Info);
    }

    #[test]
    fn usb_power_from_a_fake_sysfs() {
        let root = std::env::temp_dir().join(format!("akm-doctor-{}", std::process::id()));
        let usb = root.join("devices/1-14");
        let intf = usb.join("1-14:1.0");
        std::fs::create_dir_all(intf.join("bluetooth/hci0")).unwrap();
        std::fs::create_dir_all(usb.join("power")).unwrap();
        for (n, v) in [
            ("idVendor", "8087"),
            ("idProduct", "0026"),
            ("power/control", "auto"),
            ("power/runtime_status", "active"),
            ("power/runtime_suspended_time", "1500"),
            ("power/autosuspend_delay_ms", "2000"),
        ] {
            std::fs::write(usb.join(n), format!("{v}\n")).unwrap();
        }
        std::fs::create_dir_all(root.join("class/bluetooth/hci0")).unwrap();
        std::os::unix::fs::symlink(&intf, root.join("class/bluetooth/hci0/device")).unwrap();
        let p = usb_power(&root, "hci0").unwrap();
        assert_eq!(p.id, "8087:0026");
        assert_eq!(p.control, "auto");
        assert_eq!(p.suspended_ms, 1500);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn journal_summary_counts_this_keyboard_only() {
        let l = |s: &str| s.to_string();
        let lines = vec![
            l("2026-10-01T02:55:20+02:00 PC01 bluetoothd[1164]: profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:F1: Host is down (112)"),
            l("2026-10-01T02:55:52+02:00 PC01 bluetoothd[1164]: profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:F1: Host is down (112)"),
            l("2026-10-01T02:56:00+02:00 PC01 bluetoothd[1164]: profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:FF: Host is down (112)"),
            l("2026-10-01T04:00:16+02:00 PC01 bluetoothd[1164]: profiles/input/device.c:hidp_report_req_timeout() Device AA:BB:CC:DD:EE:F1 HIDP GET_REPORT request timed out"),
            l("2026-09-27T12:59:07+02:00 PC01 bluetoothd[1164]: src/main.c:check_options() Unknown key ReconnectUUIDs for group AdvMon in /etc/bluetooth/main.conf"),
            l("2026-09-27T12:59:08+02:00 PC01 bluetoothd[1164]: Endpoint registered: sender=:1.100"),
        ];
        let s = summarize_journal(&lines, Some("AA:BB:CC:DD:EE:F1"));
        let get = |k: JournalKind| s.iter().find(|x| x.0 == k).map(|x| x.1);
        assert_eq!(get(JournalKind::PageTimeout), Some(2));
        assert_eq!(get(JournalKind::GetReportTimeout), Some(1));
        assert_eq!(get(JournalKind::ConfigIgnored), Some(1));
        let fs = journal_findings(&s);
        assert!(fs
            .iter()
            .any(|x| x.level == Level::Bad && x.text.contains("ignored")));
    }

    #[test]
    fn key_fingerprint_comparison() {
        let a = fingerprint("0x00112233445566778899AABBCCDDEEFF");
        let b = fingerprint("00112233445566778899aabbccddeeff");
        assert_eq!(a, b, "case and 0x prefix do not matter");
        assert_ne!(a, fingerprint("00112233445566778899aabbccddeef0"));
        assert!(!a.contains("0011"), "never the key itself");
        assert_eq!(key_finding(Some(&a), Some(&b), true).level, Level::Ok);
        assert_eq!(key_finding(Some(&a), Some("x"), true).level, Level::Bad);
        assert_eq!(key_finding(None, None, true).level, Level::Bad);
        assert_eq!(key_finding(None, None, false).level, Level::Info);
    }

    fn kb(paired: bool, connected: bool) -> KbFacts {
        KbFacts {
            mac: "AA:BB:CC:DD:EE:F1".into(),
            name: "Clavier de alice #1".into(),
            paired,
            bonded: paired,
            trusted: true,
            connected,
            legacy: true,
            wake_allowed: true,
            ..Default::default()
        }
    }

    #[test]
    fn verdicts() {
        let ok = keyboard_findings(Some(&kb(true, true)), Some(true));
        assert_eq!(verdict(&ok, Some("connected"), true).0, Level::Ok);
        let asleep = keyboard_findings(Some(&kb(true, false)), Some(true));
        let v = verdict(&asleep, Some("dormant"), false);
        assert_eq!(v.0, Level::Info);
        assert!(v.1.contains("press a key"));
        let v = verdict(&asleep, Some("auth-failed"), false);
        assert_eq!(v.0, Level::Bad);
        assert!(v.1.contains("akmctl repair"));
        let unpaired = keyboard_findings(Some(&kb(false, false)), Some(true));
        assert!(verdict(&unpaired, None, false).1.contains("akmctl repair"));
        let none = keyboard_findings(None, None);
        assert_eq!(verdict(&none, None, false).0, Level::Bad);
    }
}
