//! The closed list of corrections `akmctl doctor --fix` may have applied as root, and how
//! each one rewrites its file.

use crate::{tr, N_};
pub const COMMAND: &str = "doctor-fix";

/// Installed adapter rule: written by the helper, checked by `akmctl doctor`.
pub const ADAPTER_RULE_PATH: &str = "/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules";
pub const ADAPTER_RULE: &str =
    include_str!("../../../../udev/61-akm-bt-adapter-no-autosuspend.rules");
pub const ADAPTER_USB_ID: &str = "8087:0026";

pub const USAGE: &str =
    "usage: akm-helper doctor-fix <bluez-conf|upower-conf|adapter-autosuspend>... [--restart] [--dry-run]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fix {
    /// `/etc/bluetooth/main.conf`.
    BluezConf,
    /// `/etc/UPower/UPower.conf`: `NoPollBatteries = true`.
    UpowerConf,
    /// `/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules`.
    AdapterAutosuspend,
}

impl Fix {
    pub const ALL: [Fix; 3] = [Fix::BluezConf, Fix::UpowerConf, Fix::AdapterAutosuspend];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Fix::BluezConf => "bluez-conf",
            Fix::UpowerConf => "upower-conf",
            Fix::AdapterAutosuspend => "adapter-autosuspend",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Fix> {
        Fix::ALL.into_iter().find(|f| f.id() == s)
    }

    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Fix::BluezConf => {
                N_!("BlueZ main.conf: FastConnectable and [Policy] Reconnect* in their sections")
            }
            Fix::UpowerConf => N_!("UPower.conf: NoPollBatteries = true (no 30 s battery polling)"),
            Fix::AdapterAutosuspend => {
                N_!("udev rule keeping the Bluetooth adapter out of USB autosuspend")
            }
        }
    }

    #[must_use]
    pub fn service(self) -> Option<&'static str> {
        match self {
            Fix::BluezConf => Some("bluetooth.service"),
            Fix::UpowerConf => Some("upower.service"),
            Fix::AdapterAutosuspend => None,
        }
    }

    #[must_use]
    pub fn wanted(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            Fix::BluezConf => &[
                ("General", "FastConnectable", "true"),
                (
                    "Policy",
                    "ReconnectUUIDs",
                    "00001124-0000-1000-8000-00805f9b34fb",
                ),
                ("Policy", "ReconnectAttempts", "7"),
                ("Policy", "ReconnectIntervals", "1,2,4,8,16,32,64"),
            ],
            Fix::UpowerConf => &[("UPower", "NoPollBatteries", "true")],
            Fix::AdapterAutosuspend => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub fixes: Vec<Fix>,
    pub restart: bool,
    pub dry_run: bool,
}

impl Request {
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        let mut a: Vec<String> = self.fixes.iter().map(|f| f.id().to_string()).collect();
        if self.restart {
            a.push("--restart".into());
        }
        if self.dry_run {
            a.push("--dry-run".into());
        }
        a
    }
}

/// Parse the arguments of `doctor-fix`.
///
/// # Errors
///
/// A message naming the invalid argument.
pub fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Request, String> {
    let mut rest: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let dry_run = rest.last() == Some(&"--dry-run");
    if dry_run {
        rest.pop();
    }
    let restart = rest.last() == Some(&"--restart");
    if restart {
        rest.pop();
    }
    if rest.is_empty() || rest.len() > Fix::ALL.len() {
        return Err(USAGE.into());
    }
    let mut fixes = Vec::new();
    for word in rest {
        let fix = Fix::parse(word).ok_or_else(|| {
            tr!(
                "unknown correction {word}; {USAGE}",
                word = format!("{word:?}"),
                USAGE = USAGE
            )
        })?;
        if fixes.contains(&fix) {
            return Err(tr!("{word} given twice", word = word));
        }
        fixes.push(fix);
    }
    if restart && fixes.iter().all(|f| f.service().is_none()) {
        return Err(tr!(
            "--restart: none of these corrections has a service to restart"
        ));
    }
    Ok(Request {
        fixes,
        restart,
        dry_run,
    })
}

fn section_of(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('[')?
        .strip_suffix(']')
        .map(str::trim)
}

fn key_of(line: &str) -> Option<&str> {
    let l = line.trim_start();
    if l.starts_with('#') || l.starts_with(';') || l.starts_with('[') {
        return None;
    }
    let (k, _) = l.split_once('=')?;
    let k = k.trim_end();
    (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some(k)
}

fn value_of(line: &str) -> &str {
    line.split_once('=').map_or("", |(_, v)| v.trim())
}

pub fn set_keys(text: &str, wanted: &[(&str, &str, &str)]) -> String {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    for &(section, key, value) in wanted {
        let mut cur: Option<String> = None;
        let mut seen = false;
        let mut header_at = None;
        let mut out: Vec<String> = Vec::with_capacity(lines.len() + 3);
        for line in lines {
            if let Some(s) = section_of(&line) {
                if s == section && header_at.is_none() {
                    header_at = Some(out.len());
                }
                cur = Some(s.to_string());
                out.push(line);
                continue;
            }
            if key_of(&line) != Some(key) {
                out.push(line);
                continue;
            }
            if cur.as_deref() != Some(section) || seen {
                continue; // misplaced, or a duplicate
            }
            seen = true;
            if value_of(&line) == value {
                out.push(line);
            } else {
                out.push(format!("{key} = {value}\n"));
            }
        }
        if !seen {
            let entry = format!("{key} = {value}\n");
            if let Some(i) = header_at {
                if !out[i].ends_with('\n') {
                    out[i].push('\n');
                }
                out.insert(i + 1, entry);
            } else {
                if let Some(last) = out.last_mut() {
                    if !last.ends_with('\n') {
                        last.push('\n');
                    }
                    out.push("\n".into());
                }
                out.push(format!("[{section}]\n"));
                out.push(entry);
            }
        }
        lines = out;
    }
    lines.concat()
}

#[must_use]
pub fn changes(old: &str, new: &str) -> Vec<String> {
    let mut old_lines: Vec<&str> = old.lines().collect();
    let mut added = Vec::new();
    for l in new.lines() {
        match old_lines.iter().position(|o| *o == l) {
            Some(i) => {
                old_lines.remove(i);
            }
            None => added.push(l),
        }
    }
    old_lines
        .into_iter()
        .map(|l| format!("- {l}"))
        .chain(added.into_iter().map(|l| format!("+ {l}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUEZ_DONE: &str = "[General]\nFastConnectable = true\n\n[Policy]\nReconnectIntervals = 1,2,4,8,16,32,64\nReconnectAttempts = 7\nReconnectUUIDs = 00001124-0000-1000-8000-00805f9b34fb\n";

    const BLUEZ_CASES: &[(&str, &str)] = &[
        ("", BLUEZ_DONE),
        (
            "[General]\n#FastConnectable = false\nName = BlueZ\n\n[Policy]\n#ReconnectAttempts=7\nAutoEnable=true\n",
            "[General]\nFastConnectable = true\n#FastConnectable = false\nName = BlueZ\n\n[Policy]\nReconnectIntervals = 1,2,4,8,16,32,64\nReconnectAttempts = 7\nReconnectUUIDs = 00001124-0000-1000-8000-00805f9b34fb\n#ReconnectAttempts=7\nAutoEnable=true\n",
        ),
        (
            "[General]\nFastConnectable=false\n[AdvMon]\nReconnectAttempts = 3\nReconnectIntervals=1,2\n[Policy]\nAutoEnable=true",
            "[General]\nFastConnectable = true\n[AdvMon]\n[Policy]\nReconnectIntervals = 1,2,4,8,16,32,64\nReconnectAttempts = 7\nReconnectUUIDs = 00001124-0000-1000-8000-00805f9b34fb\nAutoEnable=true",
        ),
        (
            "[General]\nName = x",
            "[General]\nFastConnectable = true\nName = x\n\n[Policy]\nReconnectIntervals = 1,2,4,8,16,32,64\nReconnectAttempts = 7\nReconnectUUIDs = 00001124-0000-1000-8000-00805f9b34fb\n",
        ),
        (BLUEZ_DONE, BLUEZ_DONE),
        (
            "[Policy]\nReconnectAttempts=1\nReconnectAttempts=2\n[General]\n FastConnectable = TRUE \n",
            "[Policy]\nReconnectIntervals = 1,2,4,8,16,32,64\nReconnectUUIDs = 00001124-0000-1000-8000-00805f9b34fb\nReconnectAttempts = 7\n[General]\nFastConnectable = true\n",
        ),
        (
            "[General]\nFastConnectable=true\n[Policy]\nReconnectAttempts=7\nReconnectUUIDs=00001124-0000-1000-8000-00805f9b34fb\nReconnectIntervals=1,2,4,8,16,32,64\n",
            "[General]\nFastConnectable=true\n[Policy]\nReconnectAttempts=7\nReconnectUUIDs=00001124-0000-1000-8000-00805f9b34fb\nReconnectIntervals=1,2,4,8,16,32,64\n",
        ),
    ];

    #[test]
    fn bluez_conf_is_rewritten_as_expected_and_only_there() {
        for (before, after) in BLUEZ_CASES {
            assert_eq!(
                set_keys(before, Fix::BluezConf.wanted()),
                *after,
                "{before:?}"
            );
        }
    }

    #[test]
    fn every_correction_is_idempotent() {
        let extra = [
            "[UPower]\nEnableWattsUpPro=false\n#NoPollBatteries=false\n",
            "garbage\n=\n[\n[]\n x = 1",
            "[General]\r\nFastConnectable = false\r\n",
        ];
        for fix in [Fix::BluezConf, Fix::UpowerConf] {
            for before in BLUEZ_CASES.iter().map(|c| c.0).chain(extra) {
                let once = set_keys(before, fix.wanted());
                assert_eq!(set_keys(&once, fix.wanted()), once, "{fix:?} {before:?}");
                for (section, key, value) in fix.wanted() {
                    let mut cur = "";
                    let mut hits = Vec::new();
                    for l in once.lines() {
                        if let Some(s) = section_of(l) {
                            cur = s;
                        } else if key_of(l) == Some(key) {
                            hits.push((cur, value_of(l)));
                        }
                    }
                    assert_eq!(hits, [(*section, *value)], "{fix:?} {key} in {once:?}");
                }
            }
        }
    }

    #[test]
    fn upower_conf() {
        assert_eq!(
            set_keys(
                "[UPower]\nEnableWattsUpPro=false\n#NoPollBatteries=false\n",
                Fix::UpowerConf.wanted()
            ),
            "[UPower]\nNoPollBatteries = true\nEnableWattsUpPro=false\n#NoPollBatteries=false\n"
        );
        assert_eq!(
            set_keys(
                "[UPower]\nNoPollBatteries=false\n",
                Fix::UpowerConf.wanted()
            ),
            "[UPower]\nNoPollBatteries = true\n"
        );
    }

    #[test]
    fn untouched_lines_keep_their_bytes() {
        let before = "; note\r\n[General]\r\n\tName = \u{e9}t\u{e9}  \r\n[Other]\nReconnect = keep me (another key)\n";
        let after = set_keys(before, Fix::BluezConf.wanted());
        for l in before.split_inclusive('\n') {
            assert!(after.contains(l), "{l:?} lost in {after:?}");
        }
    }

    #[test]
    fn arguments_are_a_closed_list() {
        let ok = |a: &[&str]| parse_args(a).unwrap();
        assert_eq!(
            ok(&["bluez-conf"]),
            Request {
                fixes: vec![Fix::BluezConf],
                restart: false,
                dry_run: false
            }
        );
        assert_eq!(
            ok(&[
                "adapter-autosuspend",
                "bluez-conf",
                "upower-conf",
                "--restart",
                "--dry-run"
            ]),
            Request {
                fixes: vec![Fix::AdapterAutosuspend, Fix::BluezConf, Fix::UpowerConf],
                restart: true,
                dry_run: true
            }
        );
        assert!(ok(&["upower-conf", "--dry-run"]).dry_run);
        for bad in [
            &[][..],
            &["--dry-run"],
            &["--restart"],
            &["bluez-conf", "bluez-conf"],
            &["bluez-conf", "--dry-run", "--restart"],
            &["bluez-conf", "--dry-run", "--dry-run"],
            &["--dry-run", "bluez-conf"],
            &["adapter-autosuspend", "--restart"],
            &["bluez-conf", "/etc/passwd"],
            &["/etc/bluetooth/main.conf"],
            &["bluez-conf", "--file=/etc/shadow"],
            &["Bluez-Conf"],
            &["bluez-conf "],
            &["bluez-conf", ""],
            &["bluez-conf;reboot"],
            &[
                "bluez-conf",
                "upower-conf",
                "adapter-autosuspend",
                "bluez-conf",
            ],
            &["all"],
            &["sh", "-c", "id"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
        for fixes in [
            vec![Fix::BluezConf],
            vec![Fix::UpowerConf, Fix::AdapterAutosuspend],
            Fix::ALL.to_vec(),
        ] {
            for (restart, dry_run) in [(false, false), (true, false), (false, true), (true, true)] {
                let restart = restart && fixes.iter().any(|f| f.service().is_some());
                let r = Request {
                    fixes: fixes.clone(),
                    restart,
                    dry_run,
                };
                assert_eq!(parse_args(&r.args()), Ok(r));
            }
        }
    }

    #[test]
    fn the_rule_compiled_in_is_the_one_of_the_tree_and_targets_one_adapter() {
        assert!(ADAPTER_RULE.contains("ATTR{idVendor}==\"8087\", ATTR{idProduct}==\"0026\""));
        let active: Vec<&str> = ADAPTER_RULE
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect();
        assert_eq!(active.len(), 3, "{active:?}");
        assert!(!ADAPTER_RULE.contains("RUN"), "the rule runs no program");
        assert_eq!(ADAPTER_USB_ID, "8087:0026");
    }

    #[test]
    fn changes_lists_removed_and_added_lines() {
        assert!(
            changes("a\nb\n", "a\nb\n").is_empty(),
            "{:?}",
            changes("a\nb\n", "a\nb\n")
        );
        assert_eq!(
            changes("a\nb=1\nc\n", "a\nb = 2\nc\nd\n"),
            ["- b=1", "+ b = 2", "+ d"]
        );
    }
}
