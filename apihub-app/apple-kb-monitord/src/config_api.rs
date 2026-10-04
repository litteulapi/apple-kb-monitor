//! D-Bus `…1.Settings` of the System Settings module: whitelisted config.toml keys and akmctl runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use akm_core::fsutil::Mode;

use crate::devices::unblock;
use zbus::interface;

pub const PATH: &str = "/com/agenceapi/AppleKbMonitor1/Settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bool,
    Pct,
    Thresholds,
    Hysteresis,
    Chemistry,
}

/// The only keys the module reads or writes; every other section stays unread.
const KEYS: [(&str, &str, Kind); 10] = [
    ("alerts", "enabled", Kind::Bool),
    ("alerts", "thresholds", Kind::Thresholds),
    ("alerts", "critical", Kind::Pct),
    ("alerts", "hysteresis", Kind::Hysteresis),
    ("battery", "chemistry", Kind::Chemistry),
    ("notifications", "connection", Kind::Bool),
    ("notifications", "battery_replaced", Kind::Bool),
    ("notifications", "defer_to_powerdevil", Kind::Bool),
    ("display", "apple_percent", Kind::Bool),
    ("apple", "will_shutdown", Kind::Bool),
];

const CHEMISTRIES: [&str; 4] = ["alkaline", "nimh", "lithium", "unknown"];

fn lookup(key: &str) -> Option<(&'static str, &'static str, Kind)> {
    KEYS.iter()
        .copied()
        .find(|(s, k, _)| !FOREIGN.contains(s) && format!("{s}.{k}") == key)
}

/// The TOML text of a valid value, or why it is refused.
///
/// # Errors
/// Why the value is refused for this key.
pub fn render(key: &str, v: &serde_json::Value) -> Result<String, String> {
    let (_, _, kind) =
        lookup(key).ok_or_else(|| tr!("{key}: not a setting of the module", key = key))?;
    let pct = |x: &serde_json::Value| x.as_i64().filter(|n| (0..=99).contains(n));
    match kind {
        Kind::Bool => v
            .as_bool()
            .map(|b| b.to_string())
            .ok_or_else(|| tr!("{key}: true or false", key = key)),
        Kind::Pct => pct(v)
            .map(|n| n.to_string())
            .ok_or_else(|| tr!("{key}: integer from 0 to 99", key = key)),
        Kind::Hysteresis => v
            .as_f64()
            .filter(|f| (1.0..=20.0).contains(f))
            .map(|f| {
                if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            })
            .ok_or_else(|| tr!("{key}: number from 1 to 20", key = key)),
        Kind::Chemistry => v
            .as_str()
            .filter(|s| CHEMISTRIES.contains(s))
            .map(|s| format!("\"{s}\""))
            .ok_or_else(|| {
                tr!(
                    "{key}: one of {list}",
                    key = key,
                    list = CHEMISTRIES.join(", ")
                )
            }),
        Kind::Thresholds => {
            let a = v.as_array().filter(|a| !a.is_empty() && a.len() <= 10);
            let n: Option<Vec<i64>> =
                a.and_then(|a| a.iter().map(|x| pct(x).filter(|n| *n >= 1)).collect());
            n.map(|n| {
                format!(
                    "[{}]",
                    n.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
                )
            })
            .ok_or_else(|| tr!("{key}: 1 to 10 integers from 1 to 99", key = key))
        }
    }
}

/// Sections of other programs (secrets): never returned, logged nor written by the module.
const FOREIGN: [&str; 4] = ["ddc", "mqtt", "monitor", "brightness"];

/// Parsed file; an error names the line only, never the text (it may hold a secret).
fn table(text: &str) -> Result<toml::Table, String> {
    text.parse::<toml::Table>().map_err(|e| {
        let line = e.span().map_or(0, |r| {
            text[..r.start.min(text.len())].matches('\n').count() + 1
        });
        tr!("config.toml is not valid TOML (line {line})", line = line)
    })
}

fn section_of(line: &str) -> Option<String> {
    let t = line.trim();
    let t = t.split('#').next().unwrap_or("").trim();
    (t.starts_with('[') && !t.starts_with("[[") && t.ends_with(']'))
        .then(|| t[1..t.len() - 1].trim().to_string())
}

/// `  # comment` ending a `key = value` line (a `#` outside quotes), or "".
fn trailing_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '#') => {
                let start = line[..i].trim_end().len();
                return &line[start..];
            }
            _ => {}
        }
    }
    ""
}

/// Text with `section.key = value` set; every other line kept byte for byte.
#[must_use]
pub fn edit(text: &str, section: &str, key: &str, value: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut cur: Option<String> = None;
    let mut sec_end: Option<usize> = None;
    let mut found: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        if let Some(s) = section_of(l) {
            if cur.as_deref() == Some(section) && sec_end.is_none() {
                sec_end = Some(i);
            }
            cur = Some(s);
            continue;
        }
        let name = l.split('=').next().unwrap_or("").trim();
        if cur.as_deref() == Some(section) && l.contains('=') && name == key {
            found = Some(i);
        }
    }
    let had_section = cur.as_deref() == Some(section) || sec_end.is_some();
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let new_line = format!("{key} = {value}{eol}");
    let mut out: Vec<String> = lines.iter().map(std::string::ToString::to_string).collect();
    if let Some(i) = found {
        let l = lines[i];
        let end = if l.ends_with("\r\n") {
            "\r\n"
        } else if l.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let body = l.trim_end_matches(['\r', '\n']);
        let mut line = format!("{key} = {value}");
        let c = trailing_comment(body);
        if !c.is_empty() {
            let hash = body.len() - c.trim_start().len();
            line.push_str(&" ".repeat(hash.saturating_sub(line.len()).max(1)));
            line.push_str(c.trim_start());
        }
        out[i] = format!("{line}{end}");
    } else if had_section {
        let mut at = sec_end.unwrap_or(out.len());
        while at > 0 && out[at - 1].trim().is_empty() {
            at -= 1;
        }
        if at > 0 && !out[at - 1].ends_with('\n') {
            out[at - 1].push_str(eol);
        }
        out.insert(at, new_line);
    } else {
        if let Some(last) = out.last_mut() {
            if !last.ends_with('\n') {
                last.push_str(eol);
            }
        }
        if !out.is_empty() {
            out.push(eol.into());
        }
        out.push(format!("[{section}]{eol}"));
        out.push(new_line);
    }
    out.concat()
}

/// New text of config.toml, checked: the key reads back as written and nothing else changed.
///
/// # Errors
/// Why the value is refused or the edited file would not read back as expected.
pub fn set_in(text: &str, key: &str, v: &serde_json::Value) -> Result<String, String> {
    let (section, name, _) =
        lookup(key).ok_or_else(|| tr!("{key}: not a setting of the module", key = key))?;
    let rendered = render(key, v)?;
    let before = table(text)?;
    let new = edit(text, section, name, &rendered);
    let mut after = table(&new).map_err(|_| {
        tr!(
            "{key}: config.toml uses a form this module cannot edit safely",
            key = key
        )
    })?;
    let want: toml::Value = format!("v = {rendered}")
        .parse::<toml::Table>()
        .map_err(|e| e.to_string())?["v"]
        .clone();
    let got = after
        .get_mut(section)
        .and_then(|s| s.as_table_mut())
        .and_then(|s| s.remove(name));
    if got.as_ref() != Some(&want) {
        return Err(tr!(
            "{key}: config.toml uses a form this module cannot edit safely",
            key = key
        ));
    }
    let mut b = before;
    if let Some(s) = b.get_mut(section).and_then(|s| s.as_table_mut()) {
        s.remove(name);
    }
    let empty = |t: &toml::Table| {
        t.get(section)
            .and_then(|s| s.as_table())
            .is_some_and(toml::map::Map::is_empty)
    };
    if empty(&after) && !empty(&b) {
        after.remove(section);
    }
    if empty(&b) && after.get(section).is_none() {
        b.remove(section);
    }
    if b != after {
        return Err(tr!(
            "{key}: writing it would change other settings, nothing written",
            key = key
        ));
    }
    Ok(new)
}

/// Values of the whitelisted keys present in the file (JSON), never another section, and its path.
///
/// # Errors
/// The parse error of a malformed file.
pub fn get_in(text: &str, path: &Path) -> Result<String, String> {
    let t = table(text)?;
    let mut values = serde_json::Map::new();
    for (s, k, _) in KEYS {
        let v = t
            .get(s)
            .and_then(|x| x.get(k))
            .and_then(|x| serde_json::to_value(x).ok());
        values.insert(format!("{s}.{k}"), v.unwrap_or(serde_json::Value::Null));
    }
    Ok(serde_json::json!({
        "values": values,
        "revision": revision(text),
        "path": path.to_string_lossy(),
    })
    .to_string())
}

/// Identity of the file content, to refuse a write over a file changed since it was read.
#[must_use]
pub fn revision(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Larger than this, config.toml is not the daemon's: never read nor written over.
const MAX_SIZE: u64 = 256 * 1024;

/// D-Bus error name of a `SetConfig` refused because config.toml changed since it was read.
pub const CONFLICT: &str = "com.agenceapi.AppleKbMonitor1.Error.Conflict";

/// Refusals named under `com.agenceapi.AppleKbMonitor1.Error`.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "com.agenceapi.AppleKbMonitor1.Error")]
pub enum Refusal {
    /// config.toml changed since the client read it: read again, then retry ([`CONFLICT`]).
    Conflict(String),
}

/// `SetConfig` errors: the usual `org.freedesktop.DBus.Error.*`, or a named [`Refusal`].
#[derive(Debug)]
pub enum SettingsError {
    Fdo(zbus::fdo::Error),
    Refusal(Refusal),
}

impl From<zbus::fdo::Error> for SettingsError {
    fn from(e: zbus::fdo::Error) -> Self {
        Self::Fdo(e)
    }
}

impl From<zbus::Error> for SettingsError {
    fn from(e: zbus::Error) -> Self {
        Self::Fdo(e.into())
    }
}

impl zbus::DBusError for SettingsError {
    fn create_reply(&self, h: &zbus::message::Header<'_>) -> zbus::Result<zbus::Message> {
        match self {
            Self::Fdo(e) => e.create_reply(h),
            Self::Refusal(e) => e.create_reply(h),
        }
    }
    fn name(&self) -> zbus::names::ErrorName<'_> {
        match self {
            Self::Fdo(e) => e.name(),
            Self::Refusal(e) => e.name(),
        }
    }
    fn description(&self) -> Option<&str> {
        match self {
            Self::Fdo(e) => e.description(),
            Self::Refusal(e) => e.description(),
        }
    }
}

/// [`Settings::set_config`] on `config`, written through a symlink (stow, home-manager) to its target.
fn set_at(
    config: &Path,
    key: &str,
    v: &serde_json::Value,
    revision_read: &str,
) -> Result<String, SettingsError> {
    // One SetConfig at a time (each runs on its own thread): the revision check holds until the rename.
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = WRITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = read(config).map_err(zbus::fdo::Error::Failed)?;
    if revision(&text) != revision_read {
        return Err(SettingsError::Refusal(Refusal::Conflict(tr!(
            "config.toml changed since it was read"
        ))));
    }
    let new = set_in(&text, key, v).map_err(zbus::fdo::Error::InvalidArgs)?;
    akm_core::fsutil::write_atomic(config, new.as_bytes(), Mode::Keep(0o600))
        .map_err(|e| zbus::fdo::Error::Failed(format!("config.toml: {e}")))?;
    tracing::info!("settings: {key} written by the settings module");
    Ok(revision(&new))
}

/// akmctl argument lists the module may run, nothing else.
///
/// # Errors
/// Why the argument list is not one the module may run.
pub fn allowed(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let ok = match a.as_slice() {
        ["doctor" | "keys", "--json"]
        | ["selftest", "--json", "--no-save"]
        | ["keymap", "kde-apply"] => true,
        // Same names and ranges as `akmctl set param` itself.
        ["set", "param", p, v, "--persist"] => akm_core::keymap::kernel_param(p)
            .is_some_and(|k| akm_core::keymap::parse_param_value(k, v).is_ok()),
        ["rename", n, "--check" | "--yes"] => n
            .strip_prefix("--device-name=")
            .is_some_and(|n| akm_core::devname::validate(n).is_ok()),
        _ => false,
    };
    ok.then_some(())
        .ok_or_else(|| tr!("akmctl {cmd}: not allowed", cmd = a.first().unwrap_or(&"")))
}

/// Outcome of `akmctl rename --device-name` (exit codes of akmctl).
#[must_use]
pub fn device_name_verdict(code: i32, check_only: bool, timed_out: bool) -> &'static str {
    match (timed_out, check_only, code) {
        (true, _, _) => "timeout",
        (_, true, 0) => "check-ok",
        (_, true, _) => "check-failed",
        (_, _, 0) => "written",
        (_, _, 13) => "unverified",
        (_, _, 14) => "mismatch",
        (_, _, 15 | 101 | -1) => "uncertain",
        _ => "not-written",
    }
}

fn run(program: &Path, args: &[String], limit: Duration) -> serde_json::Value {
    match crate::privileged::run_bounded(Command::new(program).args(args), limit) {
        Ok(f) => serde_json::json!({
            "code": f.status.and_then(|s| s.code()).unwrap_or(-1),
            // Killed by a signal: `code` is then -1, this tells it from an exit code.
            "signal": f.status.and_then(|s| std::os::unix::process::ExitStatusExt::signal(&s)),
            "out": f.out,
            "err": f.err,
            "timed_out": f.status.is_none(),
        }),
        Err(e) => {
            serde_json::json!({
                "code": -1, "signal": null, "out": "", "err": e.to_string(), "timed_out": false
            })
        }
    }
}

pub struct Settings {
    pub config: PathBuf,
    pub akmctl: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            config: akm_core::config::default_path(),
            akmctl: akmctl_path(),
        }
    }
}

#[cfg(not(any(test, feature = "testbus")))]
fn akmctl_path() -> PathBuf {
    akm_core::paths::AKMCTL.into()
}

/// The KCM private-bus tests point the daemon at a stub akmctl.
#[cfg(any(test, feature = "testbus"))]
fn akmctl_path() -> PathBuf {
    std::env::var_os("AKM_AKMCTL").map_or_else(|| akm_core::paths::AKMCTL.into(), PathBuf::from)
}

fn read(path: &Path) -> Result<String, String> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_SIZE) {
        return Err(tr!(
            "config.toml is larger than {n} KiB",
            n = MAX_SIZE / 1024
        ));
    }
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("config.toml: {e}")),
    }
}

#[interface(name = "com.agenceapi.AppleKbMonitor1.Settings")]
impl Settings {
    /// `{"values": {"alerts.enabled": true|null, …}, "revision": …, "path": …}`: only the module's keys.
    async fn get_config(&self) -> zbus::fdo::Result<String> {
        let path = self.config.clone();
        unblock(move || read(&path).and_then(|t| get_in(&t, &path)))
            .await?
            .map_err(zbus::fdo::Error::Failed)
    }
    /// Sets one key (value as JSON) of the file read at `revision`; returns the new revision.
    async fn set_config(
        &self,
        key: String,
        value_json: &str,
        revision_read: String,
    ) -> Result<String, SettingsError> {
        let v: serde_json::Value = serde_json::from_str(value_json)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("{key}: {e}")))?;
        let path = self.config.clone();
        unblock(move || set_at(&path, &key, &v, &revision_read)).await?
    }
    /// One whitelisted akmctl run (arguments as a JSON array): `{"code","signal","out","err","timed_out"}` (+ `verdict`).
    async fn run_akmctl(
        &self,
        args_json: String,
        timeout_ms: u32,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        // `set param` and `rename --yes` reach pkexec.
        crate::devices::caller_uid(conn, &hdr).await?;
        let args: Vec<String> = serde_json::from_str(&args_json)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("arguments: {e}")))?;
        allowed(&args).map_err(zbus::fdo::Error::AccessDenied)?;
        let (prog, a) = (self.akmctl.clone(), args.clone());
        let limit = Duration::from_millis(u64::from(timeout_ms.clamp(1_000, 180_000)));
        let mut r = unblock(move || run(&prog, &a, limit)).await?;
        if args.first().map(String::as_str) == Some("rename") {
            let code = r["code"]
                .as_i64()
                .and_then(|c| i32::try_from(c).ok())
                .unwrap_or(-1);
            let v = device_name_verdict(code, args[2] == "--check", r["timed_out"] == true);
            r["verdict"] = v.into();
        }
        if args.first().map(String::as_str) == Some("set") {
            crate::service::emit_keymap_params(conn).await;
        }
        Ok(r.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn akmctl_env_override_is_test_only() {
        let prod = akm_core::srclint::prod_tokens(include_str!("config_api.rs"));
        let gate = "#[cfg(any(test,feature=\"testbus\"))]";
        let (_, gated) = prod.split_once("var_os(\"AKM_AKMCTL\")").unwrap();
        assert!(!gated.contains("AKM_AKMCTL"));
        let release = prod.split_once("#[cfg(not(any(test,feature=\"testbus\")))]fnakmctl_path()");
        assert!(release
            .unwrap()
            .1
            .split_once(gate)
            .unwrap()
            .0
            .contains("paths::AKMCTL.into()}"));
        let before = prod.split_once("var_os(\"AKM_AKMCTL\")").unwrap().0;
        assert!(before.rsplit_once("fn").unwrap().0.ends_with(gate));
    }

    #[test]
    fn run_reports_the_killing_signal() {
        let sh = Path::new("/bin/sh");
        let args = |s: &str| vec!["-c".to_owned(), s.to_owned()];
        let r = run(sh, &args("kill -SEGV $$"), Duration::from_secs(10));
        assert_eq!(
            (r["code"].clone(), r["signal"].clone()),
            (json!(-1), json!(11)),
            "{r}"
        );
        let r = run(sh, &args("exit 3"), Duration::from_secs(10));
        assert_eq!(
            (r["code"].clone(), r["signal"].clone()),
            (json!(3), json!(null)),
            "{r}"
        );
        let r = run(Path::new("/nonexistent/akm"), &[], Duration::from_secs(10));
        assert_eq!(r.get("signal"), Some(&json!(null)), "{r}");
    }

    const FILE: &str = "# my settings\n[mqtt]\npassword = \"s3cret\" # keep\n\n[alerts]\nenabled = false # off\nthresholds = [20, 10]\n\n[battery]\nchemistry = \"nimh\"\n";

    #[test]
    fn only_the_key_changes_and_comments_stay() {
        let new = set_in(FILE, "alerts.enabled", &json!(true)).unwrap();
        assert_eq!(
            new,
            FILE.replace("enabled = false # off", "enabled = true  # off")
        );
        let new = set_in(FILE, "alerts.critical", &json!(5)).unwrap();
        assert!(
            new.contains("thresholds = [20, 10]\ncritical = 5\n\n[battery]"),
            "{new}"
        );
        let new = set_in(FILE, "apple.will_shutdown", &json!(false)).unwrap();
        assert!(
            new.starts_with(FILE) && new.ends_with("\n[apple]\nwill_shutdown = false\n"),
            "{new}"
        );
        assert!(new.contains("password = \"s3cret\" # keep"));
    }

    #[test]
    fn line_endings_and_trailing_comments_are_kept() {
        let crlf = "[notifications]\r\nconnection = true  # mine\r\n[ddc]\r\nbrightness = 40";
        let new = set_in(crlf, "notifications.connection", &json!(false)).unwrap();
        assert_eq!(
            new,
            crlf.replace("connection = true  #", "connection = false #")
        );
        let new = set_in(crlf, "notifications.battery_replaced", &json!(true)).unwrap();
        assert!(
            new.contains("# mine\r\nbattery_replaced = true\r\n[ddc]"),
            "{new:?}"
        );
        let col = set_in("[a]\n", "alerts.enabled", &json!(true)).unwrap();
        assert!(col.ends_with("[alerts]\nenabled = true\n"), "{col:?}");
        let three = "[notifications]\nconnection = true   # mine\n";
        let new = set_in(three, "notifications.connection", &json!(false)).unwrap();
        assert_eq!(new, "[notifications]\nconnection = false  # mine\n");
        assert_eq!(trailing_comment(r#"name = "a # b" # c"#), " # c");
    }

    #[test]
    fn validation_refuses_bad_values_and_foreign_keys() {
        assert!(set_in(FILE, "alerts.critical", &json!(120)).is_err());
        assert!(set_in(FILE, "alerts.hysteresis", &json!(0.5)).is_err());
        assert!(set_in(FILE, "battery.chemistry", &json!("acid")).is_err());
        assert!(set_in(FILE, "alerts.thresholds", &json!([20, 0])).is_err());
        assert!(set_in(FILE, "mqtt.password", &json!("x")).is_err());
        assert_eq!(render("alerts.hysteresis", &json!(3)).unwrap(), "3.0");
        assert_eq!(
            render("alerts.thresholds", &json!([30, 15])).unwrap(),
            "[30, 15]"
        );
    }

    #[test]
    fn a_form_it_cannot_edit_is_refused_not_mangled() {
        let dotted = "alerts.enabled = false\n";
        assert!(set_in(dotted, "alerts.enabled", &json!(true)).is_err());
        let multi = "[alerts]\nthresholds = [\n  20,\n  10,\n]\n";
        assert!(set_in(multi, "alerts.thresholds", &json!([30])).is_err());
    }

    #[test]
    fn get_returns_only_the_module_keys() {
        let j: serde_json::Value =
            serde_json::from_str(&get_in(FILE, Path::new("/c/config.toml")).unwrap()).unwrap();
        assert_eq!(j["values"]["alerts.enabled"], json!(false));
        assert_eq!(j["values"]["alerts.thresholds"], json!([20, 10]));
        assert_eq!(j["values"]["alerts.critical"], serde_json::Value::Null);
        assert!(!j.to_string().contains("s3cret") && !j.to_string().contains("mqtt"));
    }

    #[test]
    fn secrets_never_leave_the_daemon() {
        assert!(KEYS.iter().all(|(s, _, _)| !FOREIGN.contains(s)));
        for k in [
            "mqtt.password",
            "ddc.bus",
            "monitor.model",
            "brightness.min",
        ] {
            let e = set_in(FILE, k, &json!(1)).unwrap_err();
            assert!(!e.contains("s3cret"), "{e}");
        }
        let broken = "[mqtt]\npassword = \"s3cret\n[alerts]\nenabled = true\n";
        let e = get_in(broken, Path::new("/c/config.toml")).unwrap_err();
        assert!(!e.contains("s3cret") && e.contains("line 2"), "{e}");
        let e = set_in(broken, "alerts.enabled", &json!(false)).unwrap_err();
        assert!(!e.contains("s3cret"), "{e}");
    }

    #[test]
    fn write_keeps_a_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("akm-cfgapi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("config.toml");
        let rev = set_at(&p, "alerts.enabled", &json!(true), &revision("")).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        set_at(&p, "alerts.enabled", &json!(false), &rev).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn concurrent_sets_from_one_revision_yield_one_write_and_one_conflict() {
        let d = std::env::temp_dir().join(format!("akm-cfgapi-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.toml");
        for _ in 0..100 {
            std::fs::write(&p, FILE).unwrap();
            let rev = revision(FILE);
            let go = std::sync::Barrier::new(2);
            let r: Vec<_> = std::thread::scope(|s| {
                let h: Vec<_> = [
                    ("alerts.enabled", json!(true)),
                    ("alerts.critical", json!(5)),
                ]
                .into_iter()
                .map(|(k, v)| {
                    let (p, rev, go) = (&p, &rev, &go);
                    s.spawn(move || {
                        go.wait();
                        set_at(p, k, &v, rev)
                    })
                })
                .collect();
                h.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let ok: Vec<_> = r.iter().filter_map(|r| r.as_ref().ok()).collect();
            assert_eq!(ok.len(), 1, "{r:?}");
            assert!(
                r.iter()
                    .any(|r| matches!(r, Err(SettingsError::Refusal(_)))),
                "{r:?}"
            );
            assert_eq!(&revision(&std::fs::read_to_string(&p).unwrap()), ok[0]);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn set_writes_through_a_symlinked_config() {
        let d = std::env::temp_dir().join(format!("akm-cfgapi-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (real, link) = (d.join("real.toml"), d.join("config.toml"));
        std::fs::write(&real, FILE).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        set_at(&link, "alerts.enabled", &json!(true), &revision(FILE)).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_ne!(std::fs::read_to_string(&real).unwrap(), FILE);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stale_revision_is_a_named_conflict() {
        use zbus::DBusError;
        let d = std::env::temp_dir().join(format!("akm-cfgapi-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.toml");
        std::fs::write(&p, FILE).unwrap();
        let e = set_at(&p, "alerts.enabled", &json!(true), "stale").unwrap_err();
        assert_eq!(e.name(), CONFLICT);
        assert_eq!(
            e.description(),
            Some("config.toml changed since it was read")
        );
        let e = set_at(&p, "nope", &json!(true), &revision(FILE)).unwrap_err();
        assert_eq!(e.name(), "org.freedesktop.DBus.Error.InvalidArgs");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn akmctl_whitelist() {
        let a = |v: &[&str]| {
            allowed(
                &v.iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        };
        assert!(a(&["doctor", "--json"]).is_ok());
        assert!(a(&["set", "param", "fnmode", "2", "--persist"]).is_ok());
        assert!(a(&["set", "param", "evil", "2", "--persist"]).is_err());
        for v in ["5", "9", "-1", "+1", "01", " 1", "1 --x"] {
            assert!(
                a(&["set", "param", "fnmode", v, "--persist"]).is_err(),
                "{v}"
            );
        }
        assert!(a(&["set", "param", "fnmode", "2"]).is_err());
        // Every value the KCM Keys page offers (KeysPage.qml).
        let offered: &[(&str, &[&str])] = &[
            ("fnmode", &["0", "1", "2", "3", "4"]),
            ("swap_opt_cmd", &["0", "1", "2"]),
            ("swap_ctrl_cmd", &["0", "1"]),
            ("swap_fn_leftctrl", &["0", "1"]),
            ("iso_layout", &["-1", "0", "1"]),
        ];
        for (p, vs) in offered {
            for v in *vs {
                assert!(a(&["set", "param", p, v, "--persist"]).is_ok(), "{p}={v}");
            }
        }
        assert!(a(&["doctor", "--json", "; rm -rf ~"]).is_err());
        assert!(a(&["rename", "--device-name=x", "--yes", "--verbose"]).is_err());
        assert!(a(&["rename", "x", "--yes"]).is_err());
        assert!(a(&["rename", "--device-name=My keyboard", "--check"]).is_ok());
        assert!(a(&["rename", "--device-name=a\\b", "--yes"]).is_err());
        assert!(a(&["keymap", "reset"]).is_err());
        assert!(a(&["doctor", "--json", "--fix"]).is_err());
        assert_eq!(device_name_verdict(14, false, false), "mismatch");
        assert_eq!(device_name_verdict(1, true, false), "check-failed");
    }

    #[test]
    fn get_config_names_the_file_it_read() {
        let v: serde_json::Value = serde_json::from_str(
            &get_in(
                FILE,
                Path::new("/home/u/.config/apple-kb-monitor/config.toml"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            v["path"],
            json!("/home/u/.config/apple-kb-monitor/config.toml")
        );
        assert!(v["values"].is_object() && v["revision"].is_string(), "{v}");
    }
}
