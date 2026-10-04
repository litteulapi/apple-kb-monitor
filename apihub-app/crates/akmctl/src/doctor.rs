//! `akmctl doctor`: one-command diagnosis of the keyboard's Bluetooth link.

use akm_core::{tr, trn};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use akm_core::model::is_keyboard_device;
use akm_core::recovery::{classify_journal, JournalKind};
use serde_json::{json, Value};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

pub const UDEV_RULE: &str = crate::doctor_fix::ADAPTER_RULE_PATH;
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
    pub(crate) fn tag(self) -> &'static str {
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
    pub fix: Option<String>,
    pub id: String,
    pub args: Vec<String>,
}

impl Finding {
    fn id(mut self, id: &str, args: Vec<String>) -> Self {
        self.id = id.to_string();
        self.args = args;
        self
    }
}

fn f(level: Level, topic: &'static str, text: impl Into<String>, fix: Option<&str>) -> Finding {
    Finding {
        level,
        topic,
        text: text.into(),
        fix: fix.map(str::to_string),
        id: String::new(),
        args: Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperAccess {
    /// The helper is not installed.
    Missing,
    /// Executable, with its capability.
    Runnable,
    /// Not executable, or `cap_net_admin` lost: the package must be reinstalled.
    Broken,
}

impl HelperAccess {
    fn code(self) -> Option<&'static str> {
        use akm_core::rssi;
        match self {
            Self::Runnable => None,
            Self::Missing => Some(rssi::CODE_HELPER_MISSING),
            Self::Broken => Some(rssi::CODE_HELPER_FAILED),
        }
    }
}

fn signal_issue(code: &str) -> Finding {
    let (why, fix) = akm_core::rssi::explain(code);
    f(Level::Warn, "signal", &why, Some(&fix)).id(&format!("signal.{code}"), vec![])
}

pub fn rssi_finding(a: HelperAccess) -> Finding {
    match a.code() {
        None => f(
            Level::Ok,
            "signal",
            tr!("rssi-helper executable, capability cap_net_admin present"),
            None,
        )
        .id("signal.ok", vec![]),
        Some(c) => signal_issue(c),
    }
}

pub fn signal_finding(
    helper_exists: bool,
    daemon_issue: Option<&akm_core::rssi::RssiIssue>,
    this_process: HelperAccess,
) -> Finding {
    use akm_core::rssi;
    if !helper_exists {
        return signal_issue(rssi::CODE_HELPER_MISSING);
    }
    if let Some(i) = daemon_issue {
        let mut x = signal_issue(&i.code);
        x.text = tr!("the service reports: {}", x.text);
        return x;
    }
    rssi_finding(this_process)
}

fn daemon_signal() -> Option<akm_core::rssi::RssiIssue> {
    let conn = Connection::session().ok()?;
    crate::bus::get_state(&conn)
        .ok()
        .and_then(|s| s.connected.then(|| s.keyboard?.radio.rssi_error).flatten())
}

const RSSI_HELPER: &str = akm_core::rssi::HELPER_PATH;

fn helper_access() -> HelperAccess {
    use std::os::unix::ffi::OsStrExt;
    let path = std::path::Path::new(RSSI_HELPER);
    if !path.exists() {
        return HelperAccess::Missing;
    }
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL in a constant path");
    // SAFETY: access(2) on a valid NUL-terminated path, no other effect.
    let exec = unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0;
    if exec && akm_core::rssi::file_has_cap_net_admin(RSSI_HELPER) {
        HelperAccess::Runnable
    } else {
        HelperAccess::Broken
    }
}

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

pub fn check_main_conf(text: &str) -> Vec<Finding> {
    let e = ini_entries(text);
    let mut out = Vec::new();
    let misplaced: Vec<_> = e
        .iter()
        .filter(|(s, k, _)| k.starts_with("Reconnect") && s != "Policy")
        .map(|(s, k, _)| format!("{k} in [{s}]"))
        .collect();
    if !misplaced.is_empty() {
        out.push(
            f(
                Level::Bad,
                "bluez-conf",
                tr!("ignored by bluetoothd: {}", misplaced.join(", ")),
                Some("akmctl doctor --fix --restart-services"),
            )
            .id("bluez-conf.ignored", vec![misplaced.join(", ").clone()]),
        );
    } else if e
        .iter()
        .any(|(s, k, _)| s == "Policy" && k == "ReconnectAttempts")
    {
        out.push(
            f(
                Level::Ok,
                "bluez-conf",
                tr!("[Policy] Reconnect* in place"),
                None,
            )
            .id("bluez-conf.reconnect-ok", vec![]),
        );
    }
    let fast = e
        .iter()
        .find(|(s, k, _)| s == "General" && k == "FastConnectable")
        .is_some_and(|(_, _, v)| v.eq_ignore_ascii_case("true"));
    if fast {
        out.push(
            f(Level::Ok, "bluez-conf", "FastConnectable = true", None)
                .id("bluez-conf.fast-ok", vec![]),
        );
    } else {
        out.push(
            f(
                Level::Warn,
                "bluez-conf",
                tr!("FastConnectable off: slower answer to the keyboard's page after a key press"),
                Some("akmctl doctor --fix --restart-services"),
            )
            .id("bluez-conf.fast-off", vec![]),
        );
    }
    out
}

pub fn check_upower(text: &str) -> Finding {
    let no_poll = ini_entries(text)
        .iter()
        .any(|(_, k, v)| k == "NoPollBatteries" && v.eq_ignore_ascii_case("true"));
    if no_poll {
        f(
            Level::Ok,
            "upower",
            tr!("NoPollBatteries = true (no 30 s HID polling)"),
            None,
        )
        .id("upower.no-poll", vec![])
    } else {
        f(
            Level::Info,
            "upower",
            tr!("UPower reads the keyboard battery every 30 s (2 GET_REPORT over the air): the keyboard never sleeps"),
            Some(&format!(
                "{} akmctl doctor --fix --optional --restart-services",
                tr!("optional:")
            )),
        ).id("upower.polls", vec![])
    }
}

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
            tr!("adapter is not on USB (or not found)"),
            None,
        )
        .id("adapter-pm.not-usb", vec![]);
    };
    if p.control == "on" {
        f(
            Level::Ok,
            "adapter-pm",
            tr!("{} autosuspend off (power/control=on)", p.id),
            None,
        )
        .id("adapter-pm.off", vec![p.id.clone()])
    } else {
        // `--fix` installs the rule for the adapter it was written for; any other gets a template.
        let (level, fix) = if p.id == crate::doctor_fix::ADAPTER_USB_ID {
            (Level::Warn, Some("akmctl doctor --fix".to_string()))
        } else {
            (Level::Info, usb_rule_advice(&p.id))
        };
        f(
            level,
            "adapter-pm",
            tr!("{} USB autosuspend active (control={}, delay {} ms, now {}, {} s suspended since boot){}", p.id, p.control, p.delay_ms, p.runtime_status, p.suspended_ms / 1000, if rule_installed { tr!("; rule installed but not applied yet") } else { String::new() }),
            fix.as_deref(),
        ).id("adapter-pm.active", vec![p.id.clone(), p.control.clone(), p.delay_ms.clone(), p.runtime_status.clone(), (p.suspended_ms / 1000).to_string(), if rule_installed { "1" } else { "" }.to_string()])
    }
}

/// Shell lines installing a no-autosuspend udev rule for USB device `vvvv:pppp` (hex ids only),
/// under the packaged rule's name so `rule_installed` sees it.
fn usb_rule_advice(id: &str) -> Option<String> {
    let hex4 = |s: &str| s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let (v, p) = id.split_once(':').filter(|(v, p)| hex4(v) && hex4(p))?;
    Some(format!(
        "echo 'ACTION==\"add|bind|change\", SUBSYSTEM==\"usb\", ATTR{{idVendor}}==\"{v}\", ATTR{{idProduct}}==\"{p}\", TEST==\"power/control\", ATTR{{power/control}}=\"on\"' | sudo tee {UDEV_RULE} && sudo udevadm control --reload && sudo udevadm trigger --action=add --subsystem-match=usb --attr-match=idVendor={v} --attr-match=idProduct={p}"
    ))
}

pub fn summarize_journal(lines: &[String], mac: Option<&str>) -> Vec<(JournalKind, usize, String)> {
    let mut map: HashMap<JournalKind, (usize, String)> = HashMap::new();
    for l in lines {
        let about_other_device = mac.is_some_and(|m| {
            l.contains(':')
                && contains_mac(l)
                && !l
                    .to_ascii_uppercase()
                    .replace('_', ":")
                    .contains(&m.to_ascii_uppercase())
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
        let (id, lvl, text, fix) = match k {
            JournalKind::PageTimeout => (
                "journal.page-timeout",
                Level::Info,
                trn!("{n} page timeout (keyboard asleep or not listening), last {last}", "{n} page timeouts (keyboard asleep or not listening), last {last}", *n, n = n, last = last),
                None,
            ),
            JournalKind::GetReportTimeout => (
                "journal.get-report-timeout",
                Level::Warn,
                trn!("{n} HIDP GET_REPORT timeout, last {last}", "{n} HIDP GET_REPORT timeouts, last {last}", *n, n = n, last = last),
                Some(tr!("avoid HID scanning tools while measuring; see docs/RECONNECTION-PAIRING.md §3.5")),
            ),
            JournalKind::Auth => (
                "journal.auth",
                Level::Bad,
                trn!("{n} authentication / key error, last {last}", "{n} authentication / key errors, last {last}", *n, n = n, last = last),
                Some("akmctl repair".to_string()),
            ),
            JournalKind::Refused => (
                "journal.refused",
                Level::Info,
                trn!("{n} connection reset/refused by the keyboard, last {last}", "{n} connections reset/refused by the keyboard, last {last}", *n, n = n, last = last),
                None,
            ),
            JournalKind::ConfigIgnored => (
                "journal.config-ignored",
                Level::Bad,
                trn!("{n} main.conf key ignored by bluetoothd at start, last {last}", "{n} main.conf keys ignored by bluetoothd at start, last {last}", *n, n = n, last = last),
                Some("akmctl doctor --fix --restart-services".to_string()),
            ),
            JournalKind::StorageError => (
                "journal.storage-error",
                Level::Bad,
                trn!("{n} BlueZ storage write failure (a new link key may be lost at reboot), last {last}", "{n} BlueZ storage write failures (a new link key may be lost at reboot), last {last}", *n, n = n, last = last),
                Some(tr!("free disk space (btrfs: check metadata), then verify with sudo akmctl doctor")),
            ),
        };
        out.push(Finding {
            level: lvl,
            topic: "journal",
            text,
            fix,
            id: id.to_string(),
            args: vec![n.to_string(), last.clone()],
        });
    }
    out
}

#[allow(clippy::struct_excessive_bools)] // independent facts about the keyboard
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

#[allow(clippy::too_many_lines)] // one finding per keyboard condition, read top to bottom
pub fn keyboard_findings(k: Option<&KbFacts>, adapter_powered: Option<bool>) -> Vec<Finding> {
    let mut out = Vec::new();
    match adapter_powered {
        Some(true) => out.push(
            f(Level::Ok, "adapter", tr!("Bluetooth adapter powered"), None)
                .id("adapter.powered", vec![]),
        ),
        Some(false) => out.push(
            f(
                Level::Bad,
                "adapter",
                tr!("Bluetooth adapter off"),
                Some("bluetoothctl power on"),
            )
            .id("adapter.off", vec![]),
        ),
        None => out.push(
            f(
                Level::Bad,
                "adapter",
                tr!("BlueZ unreachable (bluetooth.service stopped?)"),
                Some("systemctl status bluetooth"),
            )
            .id("adapter.bluez-unreachable", vec![]),
        ),
    }
    let Some(k) = k else {
        out.push(
            f(
                Level::Bad,
                "pairing",
                tr!("no paired Apple keyboard known to BlueZ"),
                Some(&tr!("akmctl repair (pairing assistant)")),
            )
            .id("pairing.none", vec![]),
        );
        return out;
    };
    if k.paired || k.bonded {
        out.push(
            f(
                Level::Ok,
                "pairing",
                tr!(
                    "{} \u{201c}{}\u{201d} paired{}{}",
                    k.mac,
                    k.name,
                    if k.bonded {
                        tr!(", bonded")
                    } else {
                        String::new()
                    },
                    if k.legacy {
                        tr!(", legacy PIN pairing")
                    } else {
                        String::new()
                    }
                ),
                None,
            )
            .id(
                "pairing.ok",
                vec![
                    k.mac.clone(),
                    k.name.clone(),
                    if k.bonded { "1" } else { "" }.to_string(),
                    if k.legacy { "1" } else { "" }.to_string(),
                ],
            ),
        );
    } else {
        out.push(
            f(
                Level::Bad,
                "pairing",
                tr!("{} is not paired", k.mac),
                Some("akmctl repair"),
            )
            .id("pairing.not-paired", vec![k.mac.clone()]),
        );
    }
    if !k.trusted {
        out.push(
            f(
                Level::Warn,
                "pairing",
                tr!("not trusted: BlueZ may refuse the keyboard's own reconnection"),
                Some(&format!("bluetoothctl trust {}", k.mac)),
            )
            .id("pairing.not-trusted", vec![k.mac.clone()]),
        );
    }
    if k.blocked {
        out.push(
            f(
                Level::Bad,
                "pairing",
                tr!("blocked in BlueZ"),
                Some(&format!("bluetoothctl unblock {}", k.mac)),
            )
            .id("pairing.blocked", vec![k.mac.clone()]),
        );
    }
    out.push(if k.connected {
        f(Level::Ok, "link", tr!("connected"), None).id("link.connected", vec![])
    } else {
        f(
            Level::Info,
            "link",
            tr!("not connected (asleep? press a key)"),
            None,
        )
        .id("link.not-connected", vec![])
    });
    out
}

pub fn key_finding(stored: Option<&str>, kernel: Option<&str>, root: bool) -> Finding {
    if !root {
        return f(
            Level::Info,
            "link-key",
            tr!("link key not checked (needs root: sudo akmctl doctor)"),
            None,
        )
        .id("link-key.unchecked", vec![]);
    }
    match (stored, kernel) {
        (None, _) => f(
            Level::Bad,
            "link-key",
            tr!("no [LinkKey] stored for the keyboard"),
            Some("akmctl repair"),
        )
        .id("link-key.none", vec![]),
        (Some(s), Some(k)) if s == k => f(
            Level::Ok,
            "link-key",
            tr!("stored key == kernel key (fingerprint {s})", s = s),
            None,
        )
        .id("link-key.same", vec![s.to_string()]),
        (Some(s), Some(k)) => f(
            Level::Bad,
            "link-key",
            tr!(
                "stored key {s} != kernel key {k}: will change at next restart",
                s = s,
                k = k
            ),
            Some(&tr!(
                "check disk space, then akmctl repair if the keyboard is refused"
            )),
        )
        .id("link-key.differs", vec![s.to_string(), k.to_string()]),
        (Some(s), None) => f(
            Level::Warn,
            "link-key",
            tr!(
                "stored key {s}; kernel copy not readable (debugfs not mounted?)",
                s = s
            ),
            None,
        )
        .id("link-key.no-kernel", vec![s.to_string()]),
    }
}

pub fn fingerprint(hex: &str) -> String {
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

pub fn verdict_id(advice: &str) -> &'static str {
    if advice.starts_with("the pairing is refused") {
        "refused"
    } else if advice.starts_with("no usable pairing") {
        "no-pairing"
    } else if advice.starts_with("link up; to check") {
        "link-up-fix"
    } else if advice.starts_with("link up and") {
        "link-up-ok"
    } else if advice.starts_with("keyboard not connected") {
        "not-connected"
    } else {
        ""
    }
}

/// Topics of the warnings named by the "link up; to check" verdict, also its JSON args.
pub fn verdict_topics(findings: &[Finding]) -> Vec<&'static str> {
    let mut topics: Vec<&'static str> = findings
        .iter()
        .filter(|x| x.level >= Level::Warn)
        .map(|x| x.topic)
        .collect();
    topics.dedup();
    topics
}

pub fn verdict(findings: &[Finding], health: Option<&str>, connected: bool) -> (Level, String) {
    let worst = findings.iter().map(|x| x.level).max().unwrap_or(Level::Ok);
    if health == Some("auth-failed")
        || findings
            .iter()
            .any(|x| x.topic == "link-key" && x.level == Level::Bad)
    {
        return (
            Level::Bad,
            tr!("the pairing is refused or missing: run `akmctl repair`"),
        );
    }
    if findings
        .iter()
        .any(|x| x.topic == "pairing" && x.level == Level::Bad)
    {
        return (Level::Bad, tr!("no usable pairing: run `akmctl repair`"));
    }
    if connected {
        // Name what to look at instead of "the fixes above", which selftest does not print (R18).
        let topics = verdict_topics(findings);
        let msg = if topics.is_empty() {
            tr!("link up and configuration sound").to_string()
        } else {
            tr!("link up; to check: {} (akmctl doctor)", topics.join(", "))
        };
        let lvl = if topics.iter().all(|t| *t == "signal") {
            worst.min(Level::Info)
        } else {
            worst
        };
        return (lvl.max(Level::Ok), msg);
    }
    (
        worst.max(Level::Info),
        tr!(
            "keyboard not connected: press a key and wait 10 s; the daemon pages it on its own. \
Re-pair only if `akmctl doctor` reports auth-failed"
        ),
    )
}

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

/// What doctor needs of a `BlueZ` adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterFacts {
    pub powered: bool,
    pub address: Option<String>,
}

/// Each `BlueZ` adapter, by object path.
pub type Adapters = std::collections::BTreeMap<String, AdapterFacts>;

pub fn bluez_facts(conn: &Connection) -> zbus::Result<(Vec<KbFacts>, Adapters)> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
        .destination("org.bluez")?
        .path("/")?
        .build()?;
    let mut kbs = Vec::new();
    let mut adapters = Adapters::new();
    for (path, ifaces) in om.get_managed_objects()? {
        for (iface, p) in &ifaces {
            match iface.as_str() {
                "org.bluez.Adapter1" => {
                    adapters.insert(
                        path.to_string(),
                        AdapterFacts {
                            powered: pb(p, "Powered"),
                            address: ps(p, "Address").map(|a| a.to_ascii_uppercase()),
                        },
                    );
                }
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
    order_keyboards(&mut kbs);
    Ok((kbs, adapters))
}

/// Paired first, then connected, then by MAC: the same keyboard whatever the D-Bus order.
fn order_keyboards(kbs: &mut [KbFacts]) {
    kbs.sort_by_key(|k| (!(k.paired || k.bonded), !k.connected, k.mac.clone()));
}

/// Object path of the adapter the keyboard belongs to.
pub fn adapter_of(k: &KbFacts) -> &str {
    k.path
        .rsplit_once('/')
        .map_or("/org/bluez/hci0", |(a, _)| a)
}

/// Kernel name (`hciN`) of the keyboard's adapter.
pub fn hci_of(k: &KbFacts) -> &str {
    adapter_of(k).rsplit('/').next().unwrap_or("hci0")
}

/// `Powered` of the keyboard's adapter; without a keyboard, whether any adapter is powered.
pub fn adapter_powered(adapters: &Adapters, kb: Option<&KbFacts>) -> Option<bool> {
    match kb {
        Some(k) => adapters.get(adapter_of(k)).map(|a| a.powered),
        None => (!adapters.is_empty()).then(|| adapters.values().any(|a| a.powered)),
    }
}

pub fn usb_power(sys: &Path, hci: &str) -> Option<UsbPower> {
    let dev = std::fs::canonicalize(sys.join("class/bluetooth").join(hci).join("device")).ok()?;
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

/// Fingerprint of the link key `BlueZ` stored for `mac` on the adapter with address `adapter`.
fn stored_key(root: &Path, adapter: &str, mac: &str) -> Option<String> {
    let t = std::fs::read_to_string(root.join(adapter).join(mac).join("info")).ok()?;
    ini_entries(&t)
        .into_iter()
        .find(|(s, k, _)| s == "LinkKey" && k == "Key")
        .map(|(_, _, v)| fingerprint(&v))
}

fn key_fingerprints(
    mac: &str,
    hci: &str,
    adapter: Option<&str>,
) -> (Option<String>, Option<String>) {
    let stored = adapter.and_then(|a| stored_key(Path::new("/var/lib/bluetooth"), a, mac));
    let kernel = std::fs::read_to_string(format!("/sys/kernel/debug/bluetooth/{hci}/link_keys"))
        .ok()
        .and_then(|t| {
            t.lines()
                .map(|l| l.split_whitespace().collect::<Vec<_>>())
                .find(|w| w.len() >= 3 && w[0].eq_ignore_ascii_case(mac))
                .map(|w| fingerprint(w[2]))
        });
    (stored, kernel)
}

/// bluetoothd's journal of this boot, at most the last 6 hours (both said in the findings).
const JOURNAL_ARGS: [&str; 11] = [
    "-b",
    "-u",
    "bluetooth",
    "--since",
    "-6 hours",
    "--no-pager",
    "-o",
    "short-iso",
    "-n",
    "5000",
    "-q",
];

fn journal_lines() -> Option<Vec<String>> {
    let out = Command::new("journalctl")
        .args(JOURNAL_ARGS)
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

pub fn stale_finding(s: &akm_core::stale::Stale) -> Finding {
    f(Level::Warn, "versions", s.text(), Some(s.fix())).id(s.id(), s.args())
}

pub fn link_quality_finding(entry: &Value) -> Option<Finding> {
    let q = entry.get("quality").filter(|q| q.is_object())?;
    let n = |k: &str| q[k].as_u64().unwrap_or(0);
    let unstable = q["unstable"].as_bool().unwrap_or(false);
    let mut text = trn!(
        "link quality: {} disconnection in the last hour, {} in 24 h, {} in 7 days",
        "link quality: {} disconnections in the last hour, {} in 24 h, {} in 7 days",
        n("disconnects_last_hour"),
        n("disconnects_last_hour"),
        n("disconnects_last_day"),
        n("disconnects_7d")
    );
    if let Some(s) = q.get("signal_7d").filter(|s| s.is_object()) {
        text += &trn!(
            "; relative signal over 7 days: mean {} dB, min {} dB ({} measurement)",
            "; relative signal over 7 days: mean {} dB, min {} dB ({} measurements)",
            s["samples"].as_u64().unwrap_or(0),
            s["mean"].as_f64().unwrap_or(0.0),
            s["min"].as_i64().unwrap_or(0),
            s["samples"].as_u64().unwrap_or(0)
        );
    }
    if unstable {
        text += &tr!(
            " \u{2014} UNSTABLE: {} unexpected within the hour",
            n("unexpected_last_hour")
        );
    }
    let mut args: Vec<String> = [
        "disconnects_last_hour",
        "disconnects_last_day",
        "disconnects_7d",
    ]
    .iter()
    .map(|k| n(k).to_string())
    .collect();
    match q.get("signal_7d").filter(|s| s.is_object()) {
        Some(s) => args.extend([
            s["mean"].as_f64().unwrap_or(0.0).to_string(),
            s["min"].as_i64().unwrap_or(0).to_string(),
            s["samples"].as_u64().unwrap_or(0).to_string(),
        ]),
        None => args.extend([String::new(), String::new(), String::new()]),
    }
    args.push(if unstable {
        n("unexpected_last_hour").to_string()
    } else {
        String::new()
    });
    Some(
        f(
            if unstable { Level::Warn } else { Level::Ok },
            "link-quality",
            text,
            unstable
                .then(|| tr!("check the batteries, the distance, USB 3 devices near the adapter"))
                .as_deref(),
        )
        .id("link-quality", args),
    )
}

/// `None` when the daemon is not on the bus: doctor never D-Bus-activates a stopped daemon.
fn daemon_link_status() -> Option<Value> {
    let conn = Connection::session().ok()?;
    if !crate::bus::daemon_present(&conn) {
        return None;
    }
    let reply = conn
        .call_method(Some(BUS_NAME), LINK_PATH, Some(LINK_IFACE), "Status", &())
        .ok()?;
    let s: String = reply.body().deserialize().ok()?;
    serde_json::from_str(&s).ok()
}

/// The daemon's entry for keyboard `mac`; the first one only when no keyboard is named or known.
fn daemon_entry<'a>(arr: &'a [Value], mac: Option<&str>) -> Option<&'a Value> {
    match mac {
        Some(m) => arr
            .iter()
            .find(|e| e["mac"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(m))),
        None => arr.first(),
    }
}

pub struct Report {
    pub findings: Vec<Finding>,
    pub keyboard: Option<KbFacts>,
    pub health: Option<String>,
    pub verdict: (Level, String),
    pub daemon: Option<Value>,
}

#[allow(clippy::too_many_lines)] // independent facts gathered in report order
pub fn gather(mac: Option<&str>) -> Report {
    let mut fs = Vec::new();
    let (kbs, adapters) = Connection::system()
        .and_then(|c| bluez_facts(&c))
        .unwrap_or_default();
    let kb = match mac {
        Some(m) => kbs.iter().find(|k| k.mac.eq_ignore_ascii_case(m)).cloned(),
        None => kbs.first().cloned(),
    };
    let powered = adapter_powered(&adapters, kb.as_ref());
    fs.extend(keyboard_findings(kb.as_ref(), powered));
    let root = is_root();
    if let Some(k) = kb.as_ref() {
        let (s, kn) = if root {
            let address = adapters
                .get(adapter_of(k))
                .and_then(|a| a.address.as_deref());
            key_fingerprints(&k.mac, hci_of(k), address)
        } else {
            (None, None)
        };
        fs.push(key_finding(s.as_deref(), kn.as_deref(), root));
        match hidraw_of(Path::new("/sys"), &k.mac) {
            Some((dev, true)) => fs.push(
                f(Level::Ok, "hidraw", tr!("{} readable", dev.display()), None)
                    .id("hidraw.readable", vec![dev.display().to_string()]),
            ),
            Some((dev, false)) => fs.push(
                f(
                    Level::Warn,
                    "hidraw",
                    tr!("{} not readable by this user", dev.display()),
                    Some(&tr!("install udev/70-apple-kb-hidraw.rules (uaccess)")),
                )
                .id("hidraw.unreadable", vec![dev.display().to_string()]),
            ),
            None if k.connected => fs.push(
                f(
                    Level::Warn,
                    "hidraw",
                    tr!("connected but no hidraw node (uhid not created yet?)"),
                    None,
                )
                .id("hidraw.no-node", vec![]),
            ),
            None => {}
        }
    }
    let daemon_issue = daemon_signal();
    fs.push(signal_finding(
        Path::new(RSSI_HELPER).exists(),
        daemon_issue.as_ref(),
        helper_access(),
    ));
    let rule = Path::new(UDEV_RULE).exists();
    fs.push(check_usb_power(
        usb_power(Path::new("/sys"), kb.as_ref().map_or("hci0", hci_of)).as_ref(),
        rule,
    ));
    if let Ok(v) = std::fs::read_to_string("/sys/module/btusb/parameters/enable_autosuspend") {
        fs.push(
            f(
                Level::Info,
                "adapter-pm",
                tr!("btusb enable_autosuspend = {value}", value = v.trim()),
                None,
            )
            .id("adapter-pm.btusb", vec![v.trim().to_string()]),
        );
    }
    match std::fs::read_to_string(MAIN_CONF) {
        Ok(t) => fs.extend(check_main_conf(&t)),
        Err(e) => fs.push(
            f(Level::Info, "bluez-conf", format!("{MAIN_CONF}: {e}"), None).id(
                "bluez-conf.unreadable",
                vec![MAIN_CONF.to_string(), e.to_string()],
            ),
        ),
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
                    tr!("no bluetoothd error in the last 6 hours of this boot"),
                    None,
                ).id("journal.clean", vec![]));
            }
            fs.extend(journal_findings(&s));
        }
        None => fs.push(f(
            Level::Info,
            "journal",
            tr!("bluetoothd journal not readable (add yourself to group systemd-journal, or use sudo)"),
            None,
        ).id("journal.unreadable", vec![])),
    }
    let daemon = daemon_link_status();
    let entry = daemon
        .as_ref()
        .and_then(|v| daemon_entry(v.as_array()?, mac.or(kb.as_ref().map(|k| k.mac.as_str()))));
    let health = entry.and_then(|e| e["health"].as_str().map(str::to_string));
    match (&daemon, &health) {
        (None, _) => fs.push(
            f(
                Level::Warn,
                "daemon",
                tr!("apple-kb-monitord link keeper not reachable (no automatic reconnection)"),
                Some("systemctl --user restart apple-kb-monitord"),
            )
            .id("daemon.unreachable", vec![]),
        ),
        (Some(_), Some(h)) => fs.push(
            f(
                match h.as_str() {
                    "connected" | "dormant" | "suspended" => Level::Ok,
                    "unreachable" => Level::Warn,
                    _ => Level::Bad,
                },
                "daemon",
                tr!("link health: {h}", h = h),
                (h == "auth-failed").then_some("akmctl repair"),
            )
            .id("daemon.health", vec![h.clone()]),
        ),
        (Some(_), None) => fs.push(
            f(
                Level::Info,
                "daemon",
                tr!("keeper running, keyboard not tracked"),
                None,
            )
            .id("daemon.untracked", vec![]),
        ),
    }
    fs.extend(entry.and_then(link_quality_finding));
    fs.extend(akm_core::stale::scan().iter().map(stale_finding));
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
        let _ = writeln!(s, "{} {:<11} {}", x.level.tag(), x.topic, x.text);
        if let Some(fix) = &x.fix {
            let _ = writeln!(s, "{:>20}\u{2192} {fix}", "");
        }
    }
    let _ = writeln!(
        s,
        "\n{} {} {}",
        tr!("verdict:"),
        r.verdict.0.tag(),
        r.verdict.1
    );
    s
}

pub fn to_json(r: &Report) -> Value {
    let id = verdict_id(&r.verdict.1);
    let args = if id == "link-up-fix" {
        verdict_topics(&r.findings)
    } else {
        Vec::new()
    };
    json!({
        "verdict": {"level": r.verdict.0.as_str(), "advice": r.verdict.1,
                    "id": id, "args": args},
        "health": r.health,
        "keyboard": r.keyboard.as_ref().map(|k| json!({
            "mac": k.mac, "name": k.name, "paired": k.paired, "bonded": k.bonded,
            "trusted": k.trusted, "connected": k.connected, "legacy_pairing": k.legacy,
            "wake_allowed": k.wake_allowed,
        })),
        "findings": r.findings.iter().map(|x| json!({
            "level": x.level.as_str(), "topic": x.topic, "text": x.text, "fix": x.fix,
            "id": x.id, "args": x.args,
        })).collect::<Vec<_>>(),
        "daemon": r.daemon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_query_stays_within_this_boot() {
        // The finding says "this boot": the query must not reach the previous one.
        assert!(JOURNAL_ARGS.contains(&"-b"));
    }

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
                .contains("1 disconnection in the last hour, 2 in 24 h, 9 in 7 days"),
            "{}",
            fi.text
        );
        assert!(
            fi.text
                .contains("mean -2.5 dB, min -9 dB (40 measurements)"),
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
        assert!(link_quality_finding(&serde_json::json!({"mac": "x"})).is_none());
        assert!(link_quality_finding(&serde_json::json!({"quality": null})).is_none());
    }

    const BAD_CONF: &str = "[General]\nExperimental = true\n#FastConnectable = false\n\n[Policy]\n#ReconnectAttempts=7\n\n[AdvMon]\nReconnectUUIDs=00001124-0000-1000-8000-00805f9b34fb\nReconnectAttempts=7\n";
    const GOOD_CONF: &str = "[General]\nFastConnectable = true\n[Policy]\nReconnectAttempts=7\nReconnectIntervals=1,2,4\n";

    #[test]
    fn signal_access_findings() {
        assert_eq!(rssi_finding(HelperAccess::Runnable).level, Level::Ok);
        for a in [HelperAccess::Missing, HelperAccess::Broken] {
            let x = rssi_finding(a);
            assert_eq!(x.level, Level::Warn, "{a:?}");
            let fix = x.fix.unwrap();
            assert!(!fix.contains("akm ") && !fix.contains("usermod"), "{fix}");
        }
        assert!(rssi_finding(HelperAccess::Broken)
            .fix
            .unwrap()
            .contains("reinstall"));
    }

    #[test]
    fn signal_finding_follows_the_daemon_not_this_process() {
        use akm_core::rssi;
        let ok = HelperAccess::Runnable;
        assert_eq!(signal_finding(true, None, ok).level, Level::Ok);
        let issue = rssi::classify(&rssi::RssiError::Timeout);
        let x = signal_finding(true, Some(&issue), ok);
        assert_eq!(x.level, Level::Warn);
        assert!(x.text.starts_with("the service reports"), "{}", x.text);
        assert!(signal_finding(false, None, ok)
            .text
            .contains("not installed"));
    }

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
        assert!(x.text.contains("430 s") && x.fix.as_deref() == Some("akmctl doctor --fix"));
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
            l("2026-10-01T02:55:20+02:00 host bluetoothd[1164]: profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:F1: Host is down (112)"),
            l("2026-10-01T02:55:52+02:00 host bluetoothd[1164]: profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:F1: Host is down (112)"),
            l("2026-10-01T02:56:00+02:00 host bluetoothd[1164]: profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:FF: Host is down (112)"),
            l("2026-10-01T04:00:16+02:00 host bluetoothd[1164]: profiles/input/device.c:hidp_report_req_timeout() Device AA:BB:CC:DD:EE:F1 HIDP GET_REPORT request timed out"),
            l("2026-09-27T12:59:07+02:00 host bluetoothd[1164]: src/main.c:check_options() Unknown key ReconnectUUIDs for group AdvMon in /etc/bluetooth/main.conf"),
            l("2026-09-27T12:59:08+02:00 host bluetoothd[1164]: Endpoint registered: sender=:1.100"),
            l("2026-10-01T05:00:00+02:00 host bluetoothd[1164]: /org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1: Connection refused"),
            l("2026-10-01T05:00:01+02:00 host bluetoothd[1164]: /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF: Connection refused"),
        ];
        let s = summarize_journal(&lines, Some("AA:BB:CC:DD:EE:F1"));
        let get = |k: JournalKind| s.iter().find(|x| x.0 == k).map(|x| x.1);
        assert_eq!(get(JournalKind::PageTimeout), Some(2));
        assert_eq!(get(JournalKind::GetReportTimeout), Some(1));
        assert_eq!(get(JournalKind::ConfigIgnored), Some(1));
        assert_eq!(
            get(JournalKind::Refused),
            Some(1),
            "object path of this keyboard"
        );
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
            name: "Alice's keyboard #1".into(),
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
    fn an_unknown_mac_never_borrows_another_keyboard_s_daemon_entry() {
        let arr = vec![
            serde_json::json!({"mac": "AA:BB:CC:DD:EE:0A", "health": "connected"}),
            serde_json::json!({"mac": "AA:BB:CC:DD:EE:0B", "health": "dormant"}),
        ];
        let health = |m| daemon_entry(&arr, m).map(|e| e["health"].as_str().unwrap());
        assert_eq!(health(Some("aa:bb:cc:dd:ee:0b")), Some("dormant"));
        assert_eq!(health(Some("AA:BB:CC:DD:EE:0C")), None);
        assert_eq!(health(None), Some("connected"));
    }

    #[test]
    fn the_stored_key_comes_from_the_keyboard_s_adapter_only() {
        let root = std::env::temp_dir().join(format!("akm-doctor-keys-{}", std::process::id()));
        let mac = "AA:BB:CC:DD:EE:0A";
        let info = |adapter: &str, body: &str| {
            let d = root.join(adapter).join(mac);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("info"), body).unwrap();
        };
        let key = "AB".repeat(16);
        info("AA:BB:CC:DD:EE:A0", &format!("[LinkKey]\nKey={key}\n"));
        info("AA:BB:CC:DD:EE:A1", "[General]\nName=x\n");
        let fp = stored_key(&root, "AA:BB:CC:DD:EE:A0", mac);
        assert_eq!(fp, Some(fingerprint(&key)));
        assert_eq!(stored_key(&root, "AA:BB:CC:DD:EE:A1", mac), None);
        assert_eq!(stored_key(&root, "AA:BB:CC:DD:EE:A2", mac), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_keyboard_s_adapter_decides_and_the_order_is_stable() {
        let on = |path: &str, mac: &str| KbFacts {
            path: path.into(),
            mac: mac.into(),
            ..kb(true, false)
        };
        let a = on("/org/bluez/hci1/dev_AA_BB_CC_DD_EE_0B", "AA:BB:CC:DD:EE:0B");
        let b = on("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_0A", "AA:BB:CC:DD:EE:0A");
        let powered = |p| AdapterFacts {
            powered: p,
            address: None,
        };
        let adapters: Adapters = [
            ("/org/bluez/hci0".to_string(), powered(true)),
            ("/org/bluez/hci1".to_string(), powered(false)),
        ]
        .into();
        assert_eq!(adapter_powered(&adapters, Some(&a)), Some(false));
        assert_eq!(adapter_powered(&adapters, Some(&b)), Some(true));
        assert_eq!(adapter_powered(&adapters, None), Some(true));
        assert_eq!(adapter_powered(&Adapters::new(), None), None);
        assert_eq!(hci_of(&a), "hci1");
        let mut x = vec![a.clone(), b.clone()];
        let mut y = vec![b, a];
        order_keyboards(&mut x);
        order_keyboards(&mut y);
        assert_eq!(x[0].mac, "AA:BB:CC:DD:EE:0A");
        assert_eq!(y[0].mac, x[0].mac);
    }

    #[test]
    fn verdicts() {
        let ok = keyboard_findings(Some(&kb(true, true)), Some(true));
        assert_eq!(verdict(&ok, Some("connected"), true).0, Level::Ok);
        let mut sig = ok.clone();
        sig.push(rssi_finding(HelperAccess::Broken));
        let v = verdict(&sig, Some("connected"), true);
        assert_eq!(
            v,
            (
                Level::Info,
                "link up; to check: signal (akmctl doctor)".into()
            ),
            "R18"
        );
        assert_eq!(verdict_id(&v.1), "link-up-fix");
        assert!(!v.1.contains("above"));
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

    #[test]
    fn findings_carry_an_identifier() {
        use akm_core::recovery::JournalKind;
        let mut all = keyboard_findings(Some(&kb(true, true)), Some(true));
        all.extend(keyboard_findings(Some(&kb(false, false)), Some(false)));
        all.extend(keyboard_findings(None, None));
        all.push(key_finding(None, None, false));
        all.push(key_finding(Some("ab12"), Some("cd34"), true));
        all.push(rssi_finding(HelperAccess::Broken));
        all.push(rssi_finding(HelperAccess::Runnable));
        all.push(check_usb_power(None, false));
        all.extend(journal_findings(&[(JournalKind::Auth, 2, "08:00".into())]));
        let stale = akm_core::stale::Stale::PlasmaOlder {
            pid: 7,
            started: 1,
            installed: 2,
        };
        all.push(stale_finding(&stale));
        assert_eq!(
            all.last().unwrap().fix.as_deref(),
            Some("systemctl --user restart plasma-plasmashell.service")
        );
        for x in &all {
            assert!(!x.id.is_empty(), "no id: {x:?}");
            assert!(x.id.starts_with(x.topic), "{} not under {}", x.id, x.topic);
        }
        let k = all.iter().find(|x| x.id == "link-key.differs").unwrap();
        assert_eq!(k.args, ["ab12", "cd34"]);
        let j = all.iter().find(|x| x.id == "journal.auth").unwrap();
        assert_eq!(j.args, ["2", "08:00"]);
        for id in ["signal.helper_failed", "signal.ok"] {
            assert!(all.iter().any(|x| x.id == id), "{id}");
        }
        let q = link_quality_finding(&serde_json::json!({"quality": {
            "disconnects_last_hour": 4, "disconnects_last_day": 5, "disconnects_7d": 6,
            "unstable": true, "unexpected_last_hour": 4}}))
        .unwrap();
        assert_eq!(q.args, ["4", "5", "6", "", "", "", "4"]);
        let src = include_str!("doctor.rs");
        let body = &src[..src.find("#[cfg(test)]").unwrap()];
        // Calls of `f(` (not identifiers ending in f, not `fn f(`) against ids chained on a
        // call, whatever the layout.
        let calls = body
            .match_indices("f(")
            .filter(|(i, _)| !body[..*i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
            .count()
            - usize::from(body.contains("fn f("));
        let ids = body
            .match_indices(".id(")
            .filter(|(i, _)| body[..*i].trim_end().ends_with(')'))
            .count();
        assert_eq!(
            calls, ids,
            "a finding without .id(): {calls} f( for {ids} .id("
        );
        let ok = keyboard_findings(Some(&kb(true, true)), Some(true));
        for (v, id) in [
            (verdict(&ok, Some("connected"), true), "link-up-ok"),
            (verdict(&ok, Some("dormant"), false), "not-connected"),
            (verdict(&ok, Some("auth-failed"), false), "refused"),
            (
                verdict(&keyboard_findings(None, None), None, false),
                "no-pairing",
            ),
        ] {
            assert_eq!(verdict_id(&v.1), id, "{}", v.1);
        }
        let warn = vec![f(Level::Warn, "x", "y", None)];
        assert_eq!(verdict_id(&verdict(&warn, None, true).1), "link-up-fix");
        let r = Report {
            findings: all.clone(),
            keyboard: None,
            health: None,
            verdict: verdict(&ok, Some("connected"), true),
            daemon: None,
        };
        let j = to_json(&r);
        assert_eq!(j["verdict"]["id"], "link-up-ok");
        assert_eq!(j["findings"][0]["id"], all[0].id.as_str());
        assert!(j["findings"][0]["args"].is_array());
    }

    #[test]
    fn link_up_fix_verdict_carries_its_topics_as_args() {
        let fs = vec![
            f(Level::Warn, "adapter-pm", "x", None),
            f(Level::Ok, "pairing", "x", None),
            f(Level::Bad, "journal", "x", None),
        ];
        let r = Report {
            verdict: verdict(&fs, Some("connected"), true),
            findings: fs,
            keyboard: None,
            health: None,
            daemon: None,
        };
        let j = to_json(&r);
        assert_eq!(j["verdict"]["id"], "link-up-fix");
        assert_eq!(j["verdict"]["args"], json!(["adapter-pm", "journal"]));
        assert!(
            r.verdict.1.contains("adapter-pm, journal"),
            "{}",
            r.verdict.1
        );
    }

    #[test]
    fn kcm_e2e_doctor_fixture_is_what_to_json_emits() {
        let k = KbFacts {
            name: "Test keyboard".into(),
            ..kb(true, true)
        };
        let mut fs = keyboard_findings(Some(&k), Some(true));
        fs.push(check_upower(""));
        fs.push(rssi_finding(HelperAccess::Runnable));
        let journal = [
            "2026-10-01T05:00:00+02:00 host bluetoothd[1164]: /org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1: Connection refused".to_string(),
        ];
        fs.extend(journal_findings(&summarize_journal(&journal, Some(&k.mac))));
        let r = Report {
            verdict: verdict(&fs, Some("connected"), true),
            findings: fs,
            keyboard: Some(k),
            health: Some("connected".into()),
            daemon: Some(json!({"running": true})),
        };
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/e2e/fixtures/kcm/doctor.json");
        let want = serde_json::to_string_pretty(&to_json(&r)).unwrap() + "\n";
        if std::env::var_os("AKM_BLESS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            want,
            "AKM_BLESS=1 cargo test regenerates the fixture"
        );
    }

    #[test]
    fn usb_rule_template_uses_the_checked_file_and_the_packaged_actions() {
        let a = usb_rule_advice("0a12:0001").unwrap();
        assert!(a.contains(&format!("sudo tee {UDEV_RULE} ")), "{a}");
        assert!(a.contains(r#"ACTION=="add|bind|change""#), "{a}");
        assert!(usb_rule_advice("0a12:01").is_none() && usb_rule_advice("x';rm:0001").is_none());
    }
}
