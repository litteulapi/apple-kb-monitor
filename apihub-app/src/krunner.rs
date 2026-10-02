//! KRunner D-Bus runner (#98): `apihub-app --krunner`.
//!
//! Typing "clavier", "keyboard", "batterie clavier" or "fn lock" in KRunner
//! shows the battery level and autonomy, and offers to open the window, the
//! System Settings module, or to toggle the function keys.
//!
//! Thin client, like the rest of this crate: the state comes from the daemon
//! (`Json` property, `Device.FnMode`) and nothing here reads the keyboard.
//! The process is started by D-Bus activation when KRunner first asks
//! (`data/plasma-runner-applekeyboard.desktop`,
//! `plasma/krunner/com.agenceapi.AppleKbMonitor.Runner.service`), answers on
//! `org.kde.krunner1`, and leaves after [`IDLE_EXIT`] without a request. It
//! never opens a window itself: "open" is `Activate` on the window's name.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use akm_core::Snapshot;
use apple_kb_monitord::client;
use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Value};

use crate::i18n::{tr, trf};
use crate::{fn_toggle, instance, view};

/// Command-line flag handled by `main()` before any window is opened.
pub const FLAG: &str = "--krunner";
/// Bus name and object of the runner (see the two files named above).
pub const RUNNER_NAME: &str = "com.agenceapi.AppleKbMonitor.Runner";
pub const RUNNER_PATH: &str = "/runner";
/// System Settings module opened by the "settings" result.
pub const KCM: &str = "kcm_applekeyboard";
/// The process ends after this long without a request from KRunner.
pub const IDLE_EXIT: Duration = Duration::from_secs(120);
/// Longest wait for the daemon while KRunner waits for our answer.
pub const STATE_TIMEOUT: Duration = Duration::from_millis(800);

/// Ids of the results, given back by KRunner in `Run`.
pub const ID_BATTERY: &str = "battery";
pub const ID_WINDOW: &str = "window";
pub const ID_SETTINGS: &str = "settings";
pub const ID_FN: &str = "fn";

/// `KRunner::QueryMatch::CategoryRelevance`: Moderate / High.
const RELEVANCE_MODERATE: i32 = 50;
const RELEVANCE_HIGH: i32 = 70;

/// One result line: `(id, text, icon, category relevance, relevance, properties)`,
/// the wire type `(sssida{sv})` of `org.kde.krunner1.Match`.
pub type RemoteMatch = (
    String,
    String,
    String,
    i32,
    f64,
    HashMap<String, OwnedValue>,
);

/// What the runner knows when it answers a query.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// The daemon is on the session bus.
    pub daemon: bool,
    pub snap: Option<Snapshot>,
    /// `hid_apple.fnmode` from the daemon's keyboard object.
    pub fn_mode: Option<i32>,
    pub now: u64,
}

/// Words that bring every result / the battery line / the Fn line.
const WORDS_ALL: [&str; 2] = ["clavier", "keyboard"];
const WORDS_BATTERY: [&str; 4] = ["batterie", "battery", "piles", "pile"];
const WORDS_FN: [&str; 4] = ["fn lock", "fn", "touches de fonction", "function keys"];

fn normalise(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A query selects a single word when it starts it (3 letters at least, so
/// that "cla" already answers) or contains it ("batterie clavier"); a phrase
/// ("fn lock", "function keys") must be there whole.
fn hits(query: &str, words: &[&str]) -> bool {
    let typed: Vec<&str> = query.split(' ').collect();
    words.iter().any(|w| {
        let phrase: Vec<&str> = w.split(' ').collect();
        let prefix = phrase.len() == 1 && query.chars().count() >= 3 && w.starts_with(query);
        prefix || typed.windows(phrase.len()).any(|t| t == phrase.as_slice())
    })
}

fn battery_line(state: &State) -> (String, String) {
    if !state.daemon {
        return (
            tr("Apple keyboard: monitor stopped").into(),
            tr("apple-kb-monitord is not on the session bus").into(),
        );
    }
    let Some(kb) = state.snap.as_ref().and_then(|s| s.keyboard.as_ref()) else {
        return (
            tr("Apple keyboard: not connected").into(),
            tr("Open the monitor window").into(),
        );
    };
    let snap = state.snap.as_ref().expect("keyboard implies a snapshot");
    let src = view::pct_source(&kb.battery);
    let name = snap.display_name().unwrap_or(tr("Apple keyboard"));
    let text = trf("{}: battery {}", &[&name, &view::pct_text(src.value(), 0)]);
    let mut details: Vec<String> = Vec::new();
    if let Some(rem) = view::remaining_text(snap, state.now) {
        details.push(trf("autonomy {}", &[&rem]));
    }
    if let Some(v) = kb.battery.voltage.filter(|v| v.is_finite() && *v > 0.0) {
        details.push(view::volts_text(v));
    }
    if src.value().is_some() {
        details.push(src.caption().to_string());
    }
    details.push(trf(
        "read {}",
        &[&view::age_text(snap.update_age_s(state.now))],
    ));
    (text, details.join(" \u{b7} "))
}

fn remote(id: &str, text: String, icon: &str, relevance: f64, subtext: String) -> RemoteMatch {
    let mut props: HashMap<String, OwnedValue> = HashMap::new();
    if let Ok(v) = OwnedValue::try_from(Value::from(subtext)) {
        props.insert("subtext".into(), v);
    }
    let category = if relevance >= 0.9 {
        RELEVANCE_HIGH
    } else {
        RELEVANCE_MODERATE
    };
    (
        id.to_string(),
        text,
        icon.to_string(),
        category,
        relevance,
        props,
    )
}

/// The results for `query`; empty when the query is not about the keyboard.
/// Pure: no D-Bus, no clock (unit-tested).
pub fn matches(query: &str, state: &State) -> Vec<RemoteMatch> {
    let q = normalise(query);
    if q.chars().count() < 2 {
        return Vec::new();
    }
    let all = hits(&q, &WORDS_ALL);
    let battery = all || hits(&q, &WORDS_BATTERY);
    let fn_keys = all || hits(&q, &WORDS_FN);
    let mut out = Vec::new();
    if battery {
        let (text, sub) = battery_line(state);
        out.push(remote(ID_BATTERY, text, "apihub-scarab", 1.0, sub));
    }
    if fn_keys {
        // Offered only when the daemon reports a mode that can be toggled.
        if let Some(mode) = state.fn_mode.filter(|m| fn_toggle::next_mode(*m).is_some()) {
            let rel = if all { 0.7 } else { 1.0 };
            let sub = trf("Now: {}", &[&fn_toggle::mode_text(mode)]);
            out.push(remote(
                ID_FN,
                tr("Toggle the function keys").into(),
                "input-keyboard",
                rel,
                sub,
            ));
        }
    }
    if all {
        out.push(remote(
            ID_WINDOW,
            tr("Open the Apple keyboard monitor").into(),
            "window-new",
            0.8,
            tr("Battery, history, keys and diagnostics").into(),
        ));
        out.push(remote(
            ID_SETTINGS,
            tr("Apple keyboard settings").into(),
            "preferences-desktop-keyboard",
            0.6,
            tr("System Settings module").into(),
        ));
    }
    out
}

/// What `Run` does for a result id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenWindow,
    OpenSettings,
    ToggleFn,
}

pub fn action_of(match_id: &str) -> Option<Action> {
    match match_id {
        ID_BATTERY | ID_WINDOW => Some(Action::OpenWindow),
        ID_SETTINGS => Some(Action::OpenSettings),
        ID_FN => Some(Action::ToggleFn),
        _ => None,
    }
}

/// Command that opens the System Settings module (first tool found).
pub fn settings_command(have: impl Fn(&str) -> bool) -> Option<[&'static str; 2]> {
    [["systemsettings", KCM], ["kcmshell6", KCM]]
        .into_iter()
        .find(|c| have(c[0]))
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// State of the keyboard from the daemon; never starts the daemon, never
/// waits longer than [`STATE_TIMEOUT`] (KRunner is waiting for the answer).
pub fn read_state(conn: &Connection, now: u64) -> State {
    let c = conn.clone();
    instance::bounded(
        STATE_TIMEOUT,
        State {
            now,
            ..State::default()
        },
        move || {
            if !client::daemon_present(&c) {
                return State {
                    now,
                    ..State::default()
                };
            }
            let fn_mode = fn_toggle::keyboard_path(&c)
                .and_then(|p| fn_toggle::current_mode(&c, &p))
                .ok();
            State {
                daemon: true,
                snap: client::fetch_snapshot(&c).ok(),
                fn_mode,
                now,
            }
        },
    )
}

struct Runner {
    /// Connection for the calls TO the daemon and the window: not the one
    /// that serves this object, whose executor is busy while a method runs.
    conn: Connection,
    last_call: Arc<AtomicU64>,
}

impl Runner {
    fn touch(&self) {
        self.last_call.store(crate::unix_now(), Ordering::Relaxed);
    }
}

#[interface(name = "org.kde.krunner1")]
impl Runner {
    /// No secondary action: each result has one effect.
    fn actions(&self) -> Vec<(String, String, String)> {
        self.touch();
        Vec::new()
    }

    #[zbus(name = "Match")]
    fn match_(&self, query: &str) -> Vec<RemoteMatch> {
        self.touch();
        // Do not ask the daemon for a query that is not about the keyboard.
        if matches(query, &State::default()).is_empty() {
            return Vec::new();
        }
        matches(query, &read_state(&self.conn, crate::unix_now()))
    }

    fn run(&self, match_id: &str, _action_id: &str) {
        self.touch();
        let Some(action) = action_of(match_id) else {
            return;
        };
        let conn = self.conn.clone();
        // Never answer late: the polkit dialog of the toggle may stay open.
        let _ = std::thread::Builder::new()
            .name("krunner-run".into())
            .spawn(move || perform(&conn, action));
    }
}

fn perform(conn: &Connection, action: Action) {
    match action {
        Action::OpenWindow => {
            let args: HashMap<&str, Value<'_>> = HashMap::new();
            if let Err(e) = conn.call_method(
                Some(instance::APP_ID),
                instance::APP_PATH,
                Some("org.freedesktop.Application"),
                "Activate",
                &(args,),
            ) {
                eprintln!("[krunner] cannot open the window: {e}");
            }
        }
        Action::OpenSettings => match settings_command(on_path) {
            Some([bin, module]) => {
                if let Err(e) = std::process::Command::new(bin).arg(module).spawn() {
                    eprintln!("[krunner] cannot start {bin}: {e}");
                }
            }
            None => eprintln!("[krunner] neither systemsettings nor kcmshell6 found"),
        },
        Action::ToggleFn => {
            if let Err(e) = fn_toggle::toggle(conn) {
                eprintln!("[krunner] {e}");
            }
        }
    }
}

/// Serve `org.kde.krunner1` on `conn` under [`RUNNER_NAME`]; `client` is a
/// second connection, used for the outgoing calls. Returns the time of the
/// last request, for the idle exit.
pub fn serve_on(conn: &Connection, client: &Connection) -> zbus::Result<Arc<AtomicU64>> {
    let last_call = Arc::new(AtomicU64::new(crate::unix_now()));
    conn.object_server().at(
        RUNNER_PATH,
        Runner {
            conn: client.clone(),
            last_call: last_call.clone(),
        },
    )?;
    conn.request_name(RUNNER_NAME)?;
    Ok(last_call)
}

/// Entry point of `apihub-app --krunner`: exit code.
pub fn run() -> i32 {
    let served = Connection::session().and_then(|c| {
        let client = Connection::session()?;
        serve_on(&c, &client).map(|t| (c, t))
    });
    let (_conn, last_call) = match served {
        Ok(v) => v,
        // Name already owned: another runner process answers, nothing to do.
        Err(zbus::Error::NameTaken) => return 0,
        Err(e) => {
            eprintln!("apihub-app {FLAG}: {e}");
            return 1;
        }
    };
    loop {
        std::thread::sleep(Duration::from_secs(10));
        let idle = crate::unix_now().saturating_sub(last_call.load(Ordering::Relaxed));
        if idle >= IDLE_EXIT.as_secs() {
            return 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::FakeDaemon;

    const SNAPSHOT: &str = include_str!("../tests/fixtures/ui-gel-snapshot.json");

    fn state() -> State {
        let snap: Snapshot = serde_json::from_str(SNAPSHOT).unwrap();
        let now = snap.last_update + 90;
        State {
            daemon: true,
            snap: Some(snap),
            fn_mode: Some(1),
            now,
        }
    }

    fn ids(m: &[RemoteMatch]) -> Vec<&str> {
        m.iter().map(|r| r.0.as_str()).collect()
    }

    fn subtext(m: &RemoteMatch) -> String {
        m.5.get("subtext")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| String::try_from(v).ok())
            .unwrap_or_default()
    }

    #[test]
    fn keyboard_queries_give_battery_window_and_settings() {
        let s = state();
        for q in [
            "clavier",
            "Clavier",
            " keyboard ",
            "KEYBOARD",
            "cla",
            "key",
            "apple keyboard",
            "clavier apple",
        ] {
            let m = matches(q, &s);
            assert_eq!(
                ids(&m),
                [ID_BATTERY, ID_FN, ID_WINDOW, ID_SETTINGS],
                "query {q:?}"
            );
        }
        let m = matches("clavier", &s);
        let pct = view::pct_text(
            view::pct_source(&s.snap.as_ref().unwrap().keyboard.as_ref().unwrap().battery).value(),
            0,
        );
        assert!(pct.ends_with('%'), "{pct}");
        assert!(m[0].1.ends_with(&format!("battery {pct}")), "{}", m[0].1);
        assert!(subtext(&m[0]).contains("read "), "{}", subtext(&m[0]));
        assert_eq!(m[2].1, "Open the Apple keyboard monitor");
        assert_eq!(m[3].1, "Apple keyboard settings");
        // Relevances are ordered and in 0..=1; the battery line comes first.
        assert!(m.iter().all(|r| (0.0..=1.0).contains(&r.4)));
        assert!(m[0].4 > m[2].4 && m[2].4 > m[3].4);
    }

    #[test]
    fn battery_and_fn_queries_give_only_their_line() {
        let s = state();
        for q in ["batterie clavier", "battery", "piles", "bat"] {
            assert_eq!(ids(&matches(q, &s))[0], ID_BATTERY, "query {q:?}");
        }
        assert_eq!(ids(&matches("piles", &s)), [ID_BATTERY]);
        for q in ["fn lock", "fn", "Fn Lock", "touches de fonction"] {
            let m = matches(q, &s);
            assert_eq!(ids(&m), [ID_FN], "query {q:?}");
            assert_eq!(m[0].1, "Toggle the function keys");
            assert_eq!(subtext(&m[0]), "Now: media keys first");
        }
    }

    #[test]
    fn unrelated_or_too_short_queries_give_nothing() {
        let s = state();
        for q in [
            "", "c", "k", "f", "cl", "ke", "ba", "app", "apple", "firefox", "kate", "clap",
            "fnord", "batman", "calc", "function", "touch",
        ] {
            assert!(matches(q, &s).is_empty(), "query {q:?}");
        }
    }

    #[test]
    fn absent_daemon_or_keyboard_is_said_and_never_offers_the_toggle() {
        let m = matches("clavier", &State::default());
        assert_eq!(ids(&m), [ID_BATTERY, ID_WINDOW, ID_SETTINGS]);
        assert_eq!(m[0].1, "Apple keyboard: monitor stopped");
        let no_kb = State {
            daemon: true,
            snap: Some(Snapshot::default()),
            fn_mode: Some(-1),
            now: 0,
        };
        let m = matches("keyboard", &no_kb);
        assert_eq!(ids(&m), [ID_BATTERY, ID_WINDOW, ID_SETTINGS]);
        assert_eq!(m[0].1, "Apple keyboard: not connected");
        assert!(matches("fn lock", &no_kb).is_empty());
    }

    #[test]
    fn run_maps_each_result_to_one_action() {
        assert_eq!(action_of(ID_BATTERY), Some(Action::OpenWindow));
        assert_eq!(action_of(ID_WINDOW), Some(Action::OpenWindow));
        assert_eq!(action_of(ID_SETTINGS), Some(Action::OpenSettings));
        assert_eq!(action_of(ID_FN), Some(Action::ToggleFn));
        assert_eq!(action_of("rm -rf"), None);
        assert_eq!(
            settings_command(|b| b == "kcmshell6"),
            Some(["kcmshell6", KCM])
        );
        assert_eq!(settings_command(|_| true), Some(["systemsettings", KCM]));
        assert_eq!(settings_command(|_| false), None);
    }

    /// The service as KRunner sees it: `org.kde.krunner1` on a private bus,
    /// next to a fake daemon. `Match` has the wire type `a(sssida{sv})`, and
    /// running the Fn result calls the daemon's `SetFnMode`.
    #[test]
    fn dbus_service_answers_match_and_run() {
        let Some(fake) = FakeDaemon::start(2) else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        fake.set_json(SNAPSHOT);
        let server = fake.client();
        serve_on(&server, &fake.client()).unwrap();
        let client = fake.client();
        let call = |q: &str| -> Vec<RemoteMatch> {
            let reply = client
                .call_method(
                    Some(RUNNER_NAME),
                    RUNNER_PATH,
                    Some("org.kde.krunner1"),
                    "Match",
                    &(q,),
                )
                .unwrap();
            assert_eq!(reply.body().signature().unwrap().as_str(), "a(sssida{sv})");
            reply.body().deserialize().unwrap()
        };
        let m = call("clavier");
        assert_eq!(ids(&m), [ID_BATTERY, ID_FN, ID_WINDOW, ID_SETTINGS]);
        assert_eq!(m[0].1, "Test keyboard: battery 96%");
        assert_eq!(subtext(&m[1]), "Now: F1\u{2013}F12 first");
        assert!(call("firefox").is_empty());
        let actions: Vec<(String, String, String)> = client
            .call_method(
                Some(RUNNER_NAME),
                RUNNER_PATH,
                Some("org.kde.krunner1"),
                "Actions",
                &(),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert!(actions.is_empty());
        client
            .call_method(
                Some(RUNNER_NAME),
                RUNNER_PATH,
                Some("org.kde.krunner1"),
                "Run",
                &(ID_FN, ""),
            )
            .unwrap();
        let t = std::time::Instant::now();
        while fake.set_calls().is_empty() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fake.set_calls(), vec![1]);
        // A second runner cannot take the name: it would exit at once.
        let second = fake.client();
        assert!(matches!(
            serve_on(&second, &fake.client()),
            Err(zbus::Error::NameTaken)
        ));
    }

    /// #98: the files KRunner and the session bus need point at this service.
    #[test]
    fn runner_files_name_this_service() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let desktop =
            std::fs::read_to_string(root.join("data/plasma-runner-applekeyboard.desktop")).unwrap();
        for line in [
            "X-Plasma-API=DBus".to_string(),
            format!("X-Plasma-DBusRunner-Service={RUNNER_NAME}"),
            format!("X-Plasma-DBusRunner-Path={RUNNER_PATH}"),
            "X-Plasma-Request-Actions-Once=true".to_string(),
        ] {
            assert!(desktop.lines().any(|l| l == line), "missing {line}");
        }
        assert!(desktop.lines().any(|l| l.starts_with("Name[fr]=")));
        assert!(desktop.lines().any(|l| l.starts_with("Comment[fr]=")));
        // The regex that wakes the service covers every word it answers to.
        let regex = desktop
            .lines()
            .find_map(|l| l.strip_prefix("X-Plasma-Runner-Match-Regex="))
            .unwrap();
        let alternatives: Vec<&str> = regex
            .trim_start_matches("(?i)(^| )(")
            .trim_end_matches(')')
            .split('|')
            .collect();
        for w in WORDS_ALL.iter().chain(&WORDS_BATTERY).chain(&WORDS_FN) {
            assert!(
                alternatives.iter().any(|a| w.starts_with(a)),
                "{w} not woken by {regex}"
            );
        }
        let service = std::fs::read_to_string(
            root.join("plasma/krunner/com.agenceapi.AppleKbMonitor.Runner.service"),
        )
        .unwrap();
        assert!(service.lines().any(|l| l == format!("Name={RUNNER_NAME}")));
        assert!(service
            .lines()
            .any(|l| l == format!("Exec=/usr/bin/apihub-app {FLAG}")));
        let pkgbuild = std::fs::read_to_string(root.join("PKGBUILD")).unwrap();
        assert!(
            pkgbuild.contains("usr/share/krunner/dbusplugins/plasma-runner-applekeyboard.desktop")
        );
        assert!(pkgbuild.contains(&format!("usr/share/dbus-1/services/{RUNNER_NAME}.service")));
    }
}
