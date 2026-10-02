//! Desktop notifications through `org.freedesktop.Notifications`, in zbus
//! directly (no `notify-rust`: one zbus version in the tree), speaking the
//! dialect of KDE's `KNotification` so Plasma treats the daemon as a
//! configurable application (#249):
//!
//! * `desktop-entry`, `x-kde-appname` and `x-kde-eventId` hints: Plasma binds
//!   the notification to `apple-kb-monitor.notifyrc` (System Settings >
//!   Notifications > Apple Keyboard Monitor) and applies the user's popup,
//!   sound, history and Do Not Disturb settings for that event;
//! * buttons (`actions`) handled by a dedicated listener thread on
//!   `ActionInvoked`, never on the caller's thread;
//! * `replaces_id` per slot: a new battery alert replaces the previous one;
//! * texts in French or English from `LC_ALL` / `LC_MESSAGES` / `LANG`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use akm_core::alerts::{Crossing, Urgency};
use akm_core::batteries::Replacement;
use akm_core::chemistry::AlertBasis;
use akm_core::link::{self, LinkEvent};
use akm_core::reminder::{BatteryReminder, ReminderLevel};
use zbus::zvariant::Value;

/// Longest wait for one `Notify` call before it is abandoned.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Notifications waiting for the sender thread; beyond that they are dropped.
const QUEUE: usize = 16;
/// Calls abandoned and still blocked before new ones are refused.
const MAX_BLOCKED: usize = 4;

type Job = Box<dyn FnOnce() + Send>;

/// Hands notifications to a dedicated thread (#162). The caller (the
/// acquisition actor) only does a `try_send`: a silent notification server can
/// delay nothing but the notifications themselves.
pub struct Notifier {
    tx: SyncSender<Job>,
}

impl Notifier {
    pub fn new(call_timeout: Duration, max_blocked: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Job>(QUEUE);
        let spawned = thread::Builder::new()
            .name("kb-notify".into())
            .spawn(move || {
                let in_flight = Arc::new(AtomicUsize::new(0));
                for job in rx {
                    if in_flight.load(Ordering::SeqCst) >= max_blocked {
                        tracing::warn!("notification dropped: the notification server is stuck");
                        continue;
                    }
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    let (done_tx, done_rx) = mpsc::channel::<()>();
                    let counter = in_flight.clone();
                    let started = thread::Builder::new().name("kb-notify-call".into()).spawn(move || {
                        job();
                        counter.fetch_sub(1, Ordering::SeqCst);
                        let _ = done_tx.send(());
                    });
                    match started {
                        Ok(_) => {
                            if done_rx.recv_timeout(call_timeout).is_err() {
                                tracing::warn!(
                                    "notification server did not answer within {call_timeout:?}"
                                );
                            }
                        }
                        Err(_) => {
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("cannot start the notification thread: {e}");
        }
        Self { tx }
    }

    /// Queue a job; never blocks. False if it was dropped.
    pub fn submit(&self, job: impl FnOnce() + Send + 'static) -> bool {
        match self.tx.try_send(Box::new(job)) {
            Ok(()) => true,
            Err(_) => {
                tracing::warn!("notification dropped (queue full or sender gone)");
                false
            }
        }
    }
}


fn notifier() -> &'static Notifier {
    static N: OnceLock<Notifier> = OnceLock::new();
    N.get_or_init(|| Notifier::new(CALL_TIMEOUT, MAX_BLOCKED))
}

// ── Identity towards Plasma ────────────────────────────────────────────────

/// Application id: the `.desktop` file (without suffix) and the D-Bus name.
pub const DESKTOP_ENTRY: &str = "com.agenceapi.AppleKbMonitor";
/// Name of the KNotification event file (`apple-kb-monitor.notifyrc`), sent as
/// the `x-kde-appname` hint.
pub const KDE_APPNAME: &str = "apple-kb-monitor";
/// `app_name` argument of `Notify`: the display name of the application
/// (`Name=` of the `.desktop` and of `[Global]` in the notifyrc).
pub const APP_NAME: &str = "Apple Keyboard Monitor";
const APP_PATH: &str = "/com/agenceapi/AppleKbMonitor";

// ── Language ───────────────────────────────────────────────────────────────

/// Language of the texts. Only French and English exist; anything else is
/// English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// First non-empty of `LC_ALL`, `LC_MESSAGES`, `LANG` decides.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let v = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| get(k))
            .find(|v| !v.is_empty());
        match v {
            Some(v) if v.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
            _ => Lang::En,
        }
    }

    pub fn detect() -> Self {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Pick the text of this language.
    pub fn t(self, en: &'static str, fr: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::Fr => fr,
        }
    }
}

// ── Events, actions ────────────────────────────────────────────────────────

/// KNotification events: each one is a `[Event/<id>]` of the notifyrc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    BatteryLow,
    BatteryCritical,
    KeyboardAlert,
    KeyboardDisconnected,
    KeyboardReconnected,
    KeyboardOff,
    KeyboardUnreachable,
    RepairNeeded,
    KeyboardRemoved,
    BatteryEstimate,
    FirmwareUpdate,
    BatteryReminder,
    BatteryReplaced,
    Error,
}

impl Event {
    pub const ALL: [Event; 14] = [
        Event::BatteryLow,
        Event::BatteryCritical,
        Event::KeyboardAlert,
        Event::KeyboardDisconnected,
        Event::KeyboardReconnected,
        Event::KeyboardOff,
        Event::KeyboardUnreachable,
        Event::RepairNeeded,
        Event::KeyboardRemoved,
        Event::BatteryEstimate,
        Event::FirmwareUpdate,
        Event::BatteryReminder,
        Event::BatteryReplaced,
        Event::Error,
    ];

    /// Value of `x-kde-eventId` = name of the `[Event/<id>]` section.
    pub fn id(self) -> &'static str {
        match self {
            Event::BatteryLow => "BatteryLow",
            Event::BatteryCritical => "BatteryCritical",
            Event::KeyboardAlert => "KeyboardAlert",
            Event::KeyboardDisconnected => "KeyboardDisconnected",
            Event::KeyboardReconnected => "KeyboardReconnected",
            Event::KeyboardOff => "KeyboardOff",
            Event::KeyboardUnreachable => "KeyboardUnreachable",
            Event::RepairNeeded => "RepairNeeded",
            Event::KeyboardRemoved => "KeyboardRemoved",
            Event::BatteryEstimate => "BatteryEstimate",
            Event::FirmwareUpdate => "FirmwareUpdate",
            Event::BatteryReminder => "BatteryReminder",
            Event::BatteryReplaced => "BatteryReplaced",
            Event::Error => "Error",
        }
    }

    /// Notifications of one slot replace each other (`replaces_id`): a new
    /// battery alert takes the place of the previous one, a reconnection the
    /// place of the disconnection.
    pub fn slot(self) -> &'static str {
        match self {
            Event::BatteryLow
            | Event::BatteryCritical
            | Event::KeyboardAlert
            | Event::BatteryReminder
            | Event::BatteryEstimate
            | Event::BatteryReplaced => "battery",
            Event::KeyboardDisconnected
            | Event::KeyboardReconnected
            | Event::KeyboardOff
            | Event::KeyboardUnreachable => "link",
            Event::RepairNeeded | Event::KeyboardRemoved => "repair",
            Event::FirmwareUpdate => "firmware",
            Event::Error => "error",
        }
    }

    /// freedesktop.org `category` hint.
    pub fn category(self) -> &'static str {
        match self {
            Event::KeyboardReconnected => "device.added",
            Event::KeyboardDisconnected | Event::KeyboardOff | Event::KeyboardRemoved => {
                "device.removed"
            }
            Event::KeyboardUnreachable | Event::RepairNeeded | Event::Error => "device.error",
            _ => "device",
        }
    }

    /// The event whose `x-kde-eventId` is `id`.
    pub fn from_id(id: &str) -> Option<Event> {
        Event::ALL.into_iter().find(|e| e.id() == id)
    }

    /// Buttons of the event (the body click, `default`, always does the
    /// first one if it is `Open`).
    pub fn actions(self) -> &'static [Action] {
        match self {
            Event::RepairNeeded | Event::KeyboardRemoved => &[Action::Repair, Action::Open],
            Event::BatteryReminder => &[Action::Open, Action::RemindTomorrow, Action::Ignore],
            // Nothing urgent: may be shown again tomorrow (#110).
            Event::BatteryLow | Event::BatteryEstimate | Event::FirmwareUpdate => {
                &[Action::Open, Action::RemindTomorrow]
            }
            Event::Error => &[],
            _ => &[Action::Open],
        }
    }
}

/// What a button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the window (D-Bus activation of `com.agenceapi.AppleKbMonitor`).
    Open,
    /// `akmctl repair` in a terminal.
    Repair,
    /// Stop the battery reminder.
    Ignore,
    /// Show this notification again in 24 hours (#110).
    RemindTomorrow,
}

impl Action {
    /// Identifier sent in the `actions` array.
    pub fn key(self) -> &'static str {
        match self {
            Action::Open => "open",
            Action::Repair => "repair",
            Action::Ignore => "ignore",
            Action::RemindTomorrow => "remind",
        }
    }

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Action::Open => lang.t("Open", "Ouvrir"),
            Action::Repair => lang.t("Repair\u{2026}", "R\u{e9}parer\u{2026}"),
            Action::Ignore => lang.t("Ignore this reminder", "Ignorer ce rappel"),
            Action::RemindTomorrow => lang.t("Remind me tomorrow", "Me rappeler demain"),
        }
    }

    /// `ActionInvoked` key to action; `default` (click on the body) opens.
    pub fn from_key(key: &str) -> Option<Action> {
        match key {
            "default" | "open" => Some(Action::Open),
            "repair" => Some(Action::Repair),
            "ignore" => Some(Action::Ignore),
            "remind" => Some(Action::RemindTomorrow),
            _ => None,
        }
    }
}

// ── A notification, as data ────────────────────────────────────────────────

/// Everything of one `Notify` call but the id to replace.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub event: Event,
    pub summary: String,
    pub body: String,
    pub icon: String,
    pub urgency: Urgency,
    pub transient: bool,
    pub lang: Lang,
}

/// Escapes the body markup of the freedesktop notification spec (`&`, `<`,
/// `>`, `"`): Plasma renders `<a href>`, `<img>`, `<b>` in a body. Every body
/// carries external text (the keyboard name or alias, which any application
/// of the session can set with `SetAlias`, a remote device name, a firmware
/// version), so EVERY body is escaped in [`Notification::new`] (#257). The
/// tray uses the same function.
pub fn escape_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

impl Notification {
    /// `body` is plain text: it is escaped here, once, for every constructor
    /// (#257). The summary is plain text by the spec (never rendered as
    /// markup) and is kept as is.
    fn new(event: Event, lang: Lang, summary: String, body: String, icon: &str, urgency: Urgency) -> Self {
        Self {
            event,
            summary,
            body: escape_markup(&body),
            icon: icon.to_string(),
            urgency,
            // Low-urgency notifications (connection changes) are not kept in
            // the history of the server.
            transient: urgency == Urgency::Low,
            lang,
        }
    }

    /// What is kept of a notification shown later (quiet hours, "Remind me
    /// tomorrow"): the texts as shown, nothing else.
    pub fn to_stored(&self) -> akm_core::deferred::Stored {
        akm_core::deferred::Stored {
            event: self.event.id().to_string(),
            summary: self.summary.clone(),
            body: self.body.clone(),
            icon: self.icon.clone(),
            urgency: self.urgency.hint(),
            transient: self.transient,
            french: self.lang == Lang::Fr,
        }
    }

    /// Back from [`Self::to_stored`] (the body is already escaped). `None`
    /// for an event this version does not know.
    pub fn from_stored(s: &akm_core::deferred::Stored) -> Option<Self> {
        Some(Self {
            event: Event::from_id(&s.event)?,
            summary: s.summary.clone(),
            body: s.body.clone(),
            icon: s.icon.clone(),
            urgency: match s.urgency {
                0 => Urgency::Low,
                2 => Urgency::Critical,
                _ => Urgency::Normal,
            },
            transient: s.transient,
            lang: if s.french { Lang::Fr } else { Lang::En },
        })
    }

    /// `expire_timeout` in ms: critical = 0 (never expires, persistent),
    /// otherwise a delay long enough to read.
    pub fn timeout_ms(&self) -> i32 {
        match (self.urgency, self.event) {
            (Urgency::Critical, _) => 0,
            (_, Event::BatteryReminder) => 15_000,
            (Urgency::Low, _) => 6_000,
            (Urgency::Normal, _) => 12_000,
        }
    }

    /// The `actions` array of `Notify`: `default` (click) then the buttons.
    pub fn action_list(&self) -> Vec<String> {
        let acts = self.event.actions();
        let mut v = Vec::new();
        if acts.contains(&Action::Open) {
            v.push("default".to_string());
            v.push(Action::Open.label(self.lang).to_string());
        }
        for a in acts {
            v.push(a.key().to_string());
            v.push(a.label(self.lang).to_string());
        }
        v
    }

    /// The hints Plasma reads: `urgency`, `category`, the KDE trio
    /// `desktop-entry` / `x-kde-appname` / `x-kde-eventId`, `transient`.
    pub fn hints(&self) -> HashMap<&'static str, Value<'static>> {
        let mut h: HashMap<&'static str, Value<'static>> = HashMap::from([
            ("urgency", Value::U8(self.urgency.hint())),
            ("category", Value::from(self.event.category().to_string())),
            ("desktop-entry", Value::from(DESKTOP_ENTRY.to_string())),
            ("x-kde-appname", Value::from(KDE_APPNAME.to_string())),
            ("x-kde-eventId", Value::from(self.event.id().to_string())),
        ]);
        if self.transient {
            h.insert("transient", Value::Bool(true));
        }
        h
    }
}

// ── Texts (French / English) ───────────────────────────────────────────────

fn pct_s(lang: Lang, p: f64) -> String {
    match lang {
        Lang::En => format!("{p:.0}%"),
        Lang::Fr => format!("{p:.0}\u{202f}%"),
    }
}

/// Low-battery alert text.
pub fn low_battery_text(pct: f64, lang: Lang) -> (String, String) {
    (
        lang.t(
            "Apple Keyboard \u{2014} Low Battery",
            "Clavier Apple \u{2014} piles faibles",
        )
        .into(),
        match lang {
            Lang::En => format!("Battery at {:.0}% \u{2014} charge soon", pct),
            Lang::Fr => format!("Piles \u{e0} {} \u{2014} \u{e0} changer bient\u{f4}t", pct_s(lang, pct)),
        },
    )
}

pub fn low_battery(pct: f64) {
    let lang = Lang::detect();
    let (s, b) = low_battery_text(pct, lang);
    deliver(Notification::new(Event::BatteryLow, lang, s, b, "battery-caution", Urgency::Normal));
}

/// Text and icon of a threshold alert (#82). The wording says what the figure
/// is (#178): the charge estimated from the voltage and the declared
/// chemistry, or the keyboard's own indication (not a linear charge).
pub fn crossing_text(c: &Crossing, basis: AlertBasis, lang: Lang) -> (String, String, &'static str) {
    let (summary, _) = low_battery_text(c.pct, lang);
    let what = match basis {
        AlertBasis::Estimate => lang.t("Estimated charge", "Charge estim\u{e9}e"),
        AlertBasis::Firmware => lang.t("Keyboard indication", "Indication du clavier"),
    };
    let critical = c.urgency == Urgency::Critical;
    let body = match (lang, critical) {
        (Lang::En, true) => format!("{what} {:.0}% \u{2014} replace the batteries now", c.pct),
        (Lang::En, false) => format!(
            "{what} {:.0}% (below {}%) \u{2014} plan to replace the batteries",
            c.pct, c.threshold
        ),
        (Lang::Fr, true) => format!(
            "{what} {} \u{2014} changez les piles maintenant",
            pct_s(lang, c.pct)
        ),
        (Lang::Fr, false) => format!(
            "{what} {} (sous {}\u{202f}%) \u{2014} pr\u{e9}voyez de changer les piles",
            pct_s(lang, c.pct),
            c.threshold
        ),
    };
    let icon = if critical { "battery-empty" } else { "battery-caution" };
    (summary, body, icon)
}

pub fn crossing_notification(c: &Crossing, basis: AlertBasis, lang: Lang) -> Notification {
    let (s, b, icon) = crossing_text(c, basis, lang);
    let event = if c.urgency == Urgency::Critical {
        Event::BatteryCritical
    } else {
        Event::BatteryLow
    };
    Notification::new(event, lang, s, b, icon, c.urgency)
}

pub fn battery_crossing(c: &Crossing, basis: AlertBasis) {
    deliver(crossing_notification(c, basis, Lang::detect()));
}

/// Text, icon and urgency of a keyboard-driven battery alert (`0x30`, #189).
pub fn battery_state_text(
    s: akm_core::registry::BatteryState,
    lang: Lang,
) -> Option<(String, String, &'static str, Urgency)> {
    use akm_core::registry::BatteryState as B;
    match s {
        B::Low => Some((
            lang.t(
                "Apple Keyboard \u{2014} battery low (keyboard alert)",
                "Clavier Apple \u{2014} piles faibles (alerte du clavier)",
            )
            .to_string(),
            lang.t(
                "The keyboard itself reports a low battery \u{2014} plan to replace the batteries",
                "Le clavier signale lui-m\u{ea}me des piles faibles \u{2014} pr\u{e9}voyez de les changer",
            )
            .into(),
            "battery-caution",
            Urgency::Normal,
        )),
        B::Critical => Some((
            lang.t(
                "Apple Keyboard \u{2014} battery critical (keyboard alert)",
                "Clavier Apple \u{2014} piles critiques (alerte du clavier)",
            )
            .to_string(),
            lang.t(
                "The keyboard itself reports a critically low battery \u{2014} replace the batteries now",
                "Le clavier signale lui-m\u{ea}me des piles presque vides \u{2014} changez-les maintenant",
            )
            .into(),
            "battery-empty",
            Urgency::Critical,
        )),
        B::Normal | B::Invalid(_) => None,
    }
}

pub fn battery_state_notification(
    s: akm_core::registry::BatteryState,
    lang: Lang,
) -> Option<Notification> {
    let (sum, body, icon, urg) = battery_state_text(s, lang)?;
    Some(Notification::new(Event::KeyboardAlert, lang, sum, body, icon, urg))
}

/// Keyboard-driven alert (Input `0x30`, #189).
pub fn battery_state(s: akm_core::registry::BatteryState) {
    if let Some(n) = battery_state_notification(s, Lang::detect()) {
        deliver(n);
    }
}

/// Text of a connection change.
pub fn link_text(ev: &LinkEvent, lang: Lang) -> (String, String) {
    if lang == Lang::En {
        return link::text(ev);
    }
    match ev {
        LinkEvent::PoweredOff { .. } => (
            "Clavier Apple \u{2014} \u{e9}teint".into(),
            "Le clavier a \u{e9}t\u{e9} \u{e9}teint (ce n'est pas une connexion perdue)".into(),
        ),
        LinkEvent::Disconnected { .. } => (
            "Clavier Apple \u{2014} d\u{e9}connect\u{e9}".into(),
            "Le clavier n'est plus connect\u{e9}".into(),
        ),
        LinkEvent::Reconnected { pct, .. } => (
            "Clavier Apple \u{2014} reconnect\u{e9}".into(),
            match pct {
                Some(p) => format!("Reconnect\u{e9} ({})", pct_s(lang, *p)),
                None => "Reconnect\u{e9}".into(),
            },
        ),
    }
}

/// Disconnected / reconnected / switched off (#84): low urgency, transient.
pub fn link_notification(ev: &LinkEvent, lang: Lang) -> Notification {
    let (s, b) = link_text(ev, lang);
    let (event, icon) = match ev {
        LinkEvent::Disconnected { .. } => (Event::KeyboardDisconnected, "input-keyboard-virtual-off"),
        LinkEvent::PoweredOff { .. } => (Event::KeyboardOff, "input-keyboard-virtual-off"),
        LinkEvent::Reconnected { .. } => (Event::KeyboardReconnected, "input-keyboard"),
    };
    Notification::new(event, lang, s, b, icon, Urgency::Low)
}

pub fn link(ev: &LinkEvent) {
    deliver(link_notification(ev, Lang::detect()));
}

/// Text of the "new batteries" notification (#85).
pub fn replaced_text(r: &Replacement, lang: Lang) -> (String, String) {
    let before = r.pct_before.map_or("?".to_string(), |p| pct_s(lang, p));
    (
        lang.t("Apple Keyboard \u{2014} new batteries", "Clavier Apple \u{2014} piles neuves")
            .into(),
        match lang {
            Lang::En => format!(
                "Battery {before} \u{2192} {:.0}%; low-battery alerts re-armed",
                r.pct_after
            ),
            Lang::Fr => format!(
                "Piles {before} \u{2192} {} ; alertes de piles faibles r\u{e9}arm\u{e9}es",
                pct_s(lang, r.pct_after)
            ),
        },
    )
}

pub fn replaced_notification(r: &Replacement, lang: Lang) -> Notification {
    let (s, b) = replaced_text(r, lang);
    Notification::new(
        Event::BatteryReplaced,
        lang,
        s,
        b,
        "battery-full",
        Urgency::Normal,
    )
}

pub fn battery_replaced(r: &Replacement) {
    // New batteries end any snoozed reminder, and what waited to be shown
    // again about the old ones ("Remind me tomorrow", quiet hours).
    REMINDER_IGNORED.store(false, Ordering::SeqCst);
    crate::notify_policy::cancel_slot(Event::BatteryReplaced.slot());
    deliver(replaced_notification(r, Lang::detect()));
}

/// "Change the batteries" reminder, with an "Ignore this reminder" button:
/// the smoothed voltage went under one of the keyboard's own thresholds
/// (`0x60`: Low 2506 / Critical 2404 mV on an A1314), shown with the
/// percentage the tray displays (the keyboard's indication).
pub fn reminder_notification(r: &BatteryReminder, pct: Option<f64>, lang: Lang) -> Notification {
    let critical = r.level == ReminderLevel::Critical;
    let (mv, t) = (r.mv, r.threshold_mv);
    let pct = pct.filter(|p| p.is_finite());
    let body = match (lang, critical) {
        (Lang::En, _) => format!(
            "Voltage {mv}\u{202f}mV, under the keyboard's {} threshold ({t}\u{202f}mV){} \u{2014} {}",
            if critical { "Critical" } else { "Low" },
            pct.map_or(String::new(), |p| format!("; keyboard indication {p:.0}%")),
            if critical { "change the batteries now" } else { "plan to change the batteries" },
        ),
        (Lang::Fr, _) => format!(
            "Tension {mv}\u{202f}mV, sous le seuil {} du clavier ({t}\u{202f}mV){} \u{2014} {}",
            if critical { "Critique" } else { "Bas" },
            pct.map_or(String::new(), |p| format!(" ; indication du clavier {}", pct_s(lang, p))),
            if critical {
                "changez les piles maintenant"
            } else {
                "pr\u{e9}voyez de changer les piles"
            },
        ),
    };
    let summary = if critical {
        lang.t(
            "Apple Keyboard \u{2014} change the batteries now",
            "Clavier Apple \u{2014} changez les piles maintenant",
        )
    } else {
        lang.t(
            "Apple Keyboard \u{2014} time to change the batteries",
            "Clavier Apple \u{2014} pensez \u{e0} changer les piles",
        )
    };
    let (icon, urgency) = if critical {
        ("battery-empty", Urgency::Critical)
    } else {
        ("battery-caution", Urgency::Normal)
    };
    Notification::new(
        Event::BatteryReminder,
        lang,
        summary.into(),
        body,
        icon,
        urgency,
    )
}

/// The single reminder that replaces our percentage alerts when KDE
/// PowerDevil already warns about this keyboard (#254): it says it is an
/// estimate drawn from the batteries, not the figure PowerDevil sees.
pub fn estimate_notification(pct: f64, low_level: u8, lang: Lang) -> Notification {
    let body = match lang {
        Lang::En => format!(
            "Estimate from your batteries: about {:.0}% left. KDE warns separately at {low_level}% \
             of the keyboard's own indication.",
            pct
        ),
        Lang::Fr => format!(
            "Estimation selon vos piles\u{a0}: environ {} restant. KDE pr\u{e9}vient s\u{e9}par\u{e9}ment \u{e0} \
             {low_level}\u{202f}% de l'indication propre du clavier.",
            pct_s(lang, pct)
        ),
    };
    Notification::new(
        Event::BatteryEstimate,
        lang,
        lang.t(
            "Apple Keyboard \u{2014} batteries getting low",
            "Clavier Apple \u{2014} piles bient\u{f4}t faibles",
        )
        .into(),
        body,
        "battery-caution",
        Urgency::Normal,
    )
}

pub fn battery_estimate(pct: f64, low_level: u8) {
    deliver(estimate_notification(pct, low_level, Lang::detect()));
}

/// The keyboard was removed from this computer from outside (Plasma "Forget",
/// `bluetoothctl remove`) (#252): what happened and the next step.
pub fn removed_notification(name: &str, lang: Lang) -> Notification {
    let body = match lang {
        Lang::En => format!(
            "\u{201c}{name}\u{201d} was removed from this computer: switch it off and on to pair it again \
             (button Repair, or \u{201c}akmctl repair\u{201d})."
        ),
        Lang::Fr => format!(
            "\u{ab}\u{a0}{name}\u{a0}\u{bb} a \u{e9}t\u{e9} supprim\u{e9} du poste\u{a0}: \u{e9}teignez puis rallumez-le pour le \
             r\u{e9}appairer (bouton R\u{e9}parer, ou \u{ab}\u{a0}akmctl repair\u{a0}\u{bb})."
        ),
    };
    Notification::new(
        Event::KeyboardRemoved,
        lang,
        lang.t("Keyboard removed from this computer", "Clavier supprim\u{e9} du poste").into(),
        body,
        "input-keyboard-virtual-off",
        Urgency::Normal,
    )
}

pub fn keyboard_removed(name: &str) {
    deliver(removed_notification(name, Lang::detect()));
}

/// Send the reminder unless the user ignored it (until new batteries). The
/// Critical one is always sent: "Ignore this reminder" silences the Low one.
/// Returns whether it was sent.
pub fn battery_reminder(r: &BatteryReminder, pct: Option<f64>) -> bool {
    if reminder_suppressed(r.level) {
        return false;
    }
    deliver(reminder_notification(r, pct, Lang::detect()));
    true
}

/// "Ignore this reminder" applies to the Low reminder only.
pub fn reminder_suppressed(level: ReminderLevel) -> bool {
    level == ReminderLevel::Low && REMINDER_IGNORED.load(Ordering::SeqCst)
}

/// A newer firmware than the keyboard's is known.
pub fn firmware_notification(current: &str, latest: &str, lang: Lang) -> Notification {
    let body = match lang {
        Lang::En => format!("Firmware {current}; latest version known: {latest}"),
        Lang::Fr => format!("Firmware {current} ; derni\u{e8}re version connue : {latest}"),
    };
    Notification::new(
        Event::FirmwareUpdate,
        lang,
        lang.t(
            "Apple Keyboard \u{2014} firmware update known",
            "Clavier Apple \u{2014} mise \u{e0} jour du firmware connue",
        )
        .into(),
        body,
        "software-update-available",
        Urgency::Normal,
    )
}

pub fn firmware_update(current: &str, latest: &str) {
    deliver(firmware_notification(current, latest, Lang::detect()));
}

/// A notice of the link keeper (already worded in the user's language by
/// `repair::notice_text`), tied to its KDE event.
pub fn notice_notification(event: Event, summary: &str, body: &str, urgency: Urgency) -> Notification {
    Notification::new(
        event,
        Lang::detect(),
        summary.to_string(),
        body.to_string(),
        "input-keyboard",
        urgency,
    )
}

pub fn notice(event: Event, summary: &str, body: &str, urgency: Urgency) {
    deliver(notice_notification(event, summary, body, urgency));
}

/// Anything else (errors): the `Error` event, no button.
pub fn generic_notification(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) -> Notification {
    let mut n = Notification::new(
        Event::Error,
        Lang::detect(),
        summary.to_string(),
        body.to_string(),
        icon,
        urgency,
    );
    n.transient = transient;
    n
}

// ── Sending ────────────────────────────────────────────────────────────────

/// Send a critical notification on the session bus; errors are logged,
/// never fatal (a headless session has no notification server). Returns at
/// once: the call runs on the notification thread.
pub fn send(summary: &str, body: &str, icon: &str) {
    send_with(summary, body, icon, Urgency::Critical, false);
}

/// Send with an urgency; `transient` notifications are not kept in the
/// history of the notification server (connection changes).
pub fn send_with(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) {
    deliver(generic_notification(
        summary, body, icon, urgency, transient,
    ));
}

/// Queue a notification for the notification thread; never blocks. Inside
/// the quiet hours a notification that is not critical is held instead
/// (journalled, shown when they end: [`crate::notify_policy`], #91).
pub fn deliver(n: Notification) {
    let Some(n) = crate::notify_policy::admit(n) else {
        return;
    };
    notifier().submit(move || {
        deliver_blocking(&n);
    });
}

/// Show what is due now: notifications held by the quiet hours that just
/// ended, and "Remind me tomorrow" reminders (#91, #110). Cheap when nothing
/// waits; called by the acquisition loop at every pass.
pub fn tick() {
    for n in crate::notify_policy::take_due() {
        deliver(n);
    }
}

/// The blocking D-Bus call, for a generic notification (notification thread
/// only).
pub fn send_blocking(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) {
    deliver_blocking(&generic_notification(
        summary, body, icon, urgency, transient,
    ));
}

/// The blocking `Notify` call (notification thread only). Replaces the
/// previous notification of the same slot and registers the buttons.
pub fn deliver_blocking(n: &Notification) -> Option<u32> {
    if !n.event.actions().is_empty() {
        listener::ensure_started();
    }
    let replaces = lock(registry()).replace_id(n.event.slot());
    // The server may hand the click on a button back before its reply to
    // `Notify` is recorded below: the listener waits for calls in flight.
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let res = (|| -> zbus::Result<u32> {
        let conn = zbus::blocking::Connection::session()?;
        let reply = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                APP_NAME,
                replaces,
                n.icon.as_str(),
                n.summary.as_str(),
                n.body.as_str(),
                n.action_list(),
                n.hints(),
                n.timeout_ms(),
            ),
        )?;
        reply.body().deserialize()
    })();
    let sent = match res {
        Ok(id) => {
            let mut reg = lock(registry());
            reg.record(n.event.slot(), id, n.event);
            reg.remember(id, n);
            Some(id)
        }
        Err(e) => {
            tracing::warn!("notification not sent: {e}");
            None
        }
    };
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    sent
}

/// `Notify` calls sent and not recorded in the registry yet.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Longest wait of the button listener for such a call (a server that never
/// answers must not freeze the buttons of the other notifications).
const IN_FLIGHT_WAIT: Duration = Duration::from_millis(500);

/// Wait until no `Notify` call is in flight, at most [`IN_FLIGHT_WAIT`].
fn settle_in_flight() {
    let end = std::time::Instant::now() + IN_FLIGHT_WAIT;
    while IN_FLIGHT.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < end {
        thread::sleep(Duration::from_millis(5));
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ── Notifications in flight ────────────────────────────────────────────────

/// Pending notifications kept; the oldest is forgotten beyond.
const MAX_PENDING: usize = 64;

/// Set by "Ignore this reminder", cleared by new batteries.
static REMINDER_IGNORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which server id each slot shows, and which ids carry buttons.
#[derive(Debug, Default)]
pub struct Registry {
    slots: HashMap<&'static str, u32>,
    pending: HashMap<u32, Event>,
    order: VecDeque<u32>,
    tokens: HashMap<u32, String>,
    /// What each pending notification shows ("Remind me tomorrow").
    content: HashMap<u32, Notification>,
}

impl Registry {
    /// `replaces_id` for the next notification of the slot (0 = new).
    pub fn replace_id(&self, slot: &str) -> u32 {
        self.slots.get(slot).copied().unwrap_or(0)
    }

    /// The server showed `id` for `event`.
    pub fn record(&mut self, slot: &'static str, id: u32, event: Event) {
        if let Some(old) = self.slots.insert(slot, id) {
            if old != id {
                self.forget(old);
            }
        }
        if self.pending.insert(id, event).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > MAX_PENDING {
            if let Some(old) = self.order.pop_front() {
                self.forget(old);
            }
        }
    }

    /// The notification is gone (closed, or an action ran).
    pub fn forget(&mut self, id: u32) {
        self.pending.remove(&id);
        self.tokens.remove(&id);
        self.content.remove(&id);
        self.order.retain(|i| *i != id);
        self.slots.retain(|_, v| *v != id);
    }

    /// Keep what the pending notification `id` shows.
    pub fn remember(&mut self, id: u32, n: &Notification) {
        if self.pending.contains_key(&id) {
            self.content.insert(id, n.clone());
        }
    }

    /// What the pending notification `id` shows.
    pub fn content(&self, id: u32) -> Option<&Notification> {
        self.content.get(&id)
    }

    pub fn set_token(&mut self, id: u32, token: String) {
        if self.pending.contains_key(&id) {
            self.tokens.insert(id, token);
        }
    }

    /// An action was invoked on `id`: the action, if the notification is ours
    /// and offers that button, and the activation token Plasma gave.
    pub fn take_action(&mut self, id: u32, key: &str) -> Option<(Action, Option<String>)> {
        let ev = *self.pending.get(&id)?;
        let action = Action::from_key(key)?;
        if !ev.actions().contains(&action) {
            return None;
        }
        let token = self.tokens.remove(&id);
        self.forget(id);
        Some((action, token))
    }
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

// ── Buttons ────────────────────────────────────────────────────────────────

type Handler = Arc<dyn Fn(Action, Option<String>) + Send + Sync>;

fn handler_slot() -> &'static Mutex<Option<Handler>> {
    static H: OnceLock<Mutex<Option<Handler>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

/// Replace what the buttons do (tests; the default opens the window, runs the
/// repair or snoozes the reminder).
pub fn set_action_handler(h: impl Fn(Action, Option<String>) + Send + Sync + 'static) {
    *lock(handler_slot()) = Some(Arc::new(h));
}

fn run_action(action: Action, token: Option<String>, content: Option<Notification>) {
    let custom = lock(handler_slot()).clone();
    // "Remind me tomorrow" is the daemon's own doing: no handler replaces it.
    if action == Action::RemindTomorrow {
        match content.as_ref() {
            Some(n) => {
                crate::notify_policy::remind(n);
            }
            None => tracing::warn!("notification: nothing kept to remind of"),
        }
    }
    if let Some(h) = custom {
        h(action, token);
        return;
    }
    match action {
        Action::Open => open_window(token.as_deref()),
        Action::Repair => crate::repair::launch_repair(),
        Action::Ignore => {
            REMINDER_IGNORED.store(true, Ordering::SeqCst);
            crate::notify_policy::cancel_slot(Event::BatteryReminder.slot());
            tracing::info!("notification: battery reminder ignored");
        }
        Action::RemindTomorrow => {}
    }
}

/// `org.freedesktop.Application.Activate` on `com.agenceapi.AppleKbMonitor`
/// (single instance, D-Bus activation), else plain `apihub-app`.
fn open_window(token: Option<&str>) {
    let mut platform: HashMap<&str, Value<'_>> = HashMap::new();
    if let Some(t) = token {
        platform.insert("activation-token", Value::from(t));
        platform.insert("desktop-startup-id", Value::from(t));
    }
    let r = zbus::blocking::Connection::session().and_then(|c| {
        c.call_method(
            Some(DESKTOP_ENTRY),
            APP_PATH,
            Some("org.freedesktop.Application"),
            "Activate",
            &(platform,),
        )
    });
    if let Err(e) = r {
        tracing::debug!("notification: {DESKTOP_ENTRY} not activatable ({e}), running apihub-app");
        let mut cmd = std::process::Command::new("apihub-app");
        cmd.stdin(std::process::Stdio::null());
        if let Some(t) = token {
            cmd.env("XDG_ACTIVATION_TOKEN", t).env("DESKTOP_STARTUP_ID", t);
        }
        if let Err(e) = cmd.spawn() {
            tracing::warn!("notification: cannot start apihub-app: {e}");
        }
    }
}

/// The dedicated thread listening to `ActionInvoked`, `NotificationClosed` and
/// Plasma's `ActivationToken` on its own connection: nothing the caller or
/// the sender thread does waits for it, nor it for them.
mod listener {
    use super::*;
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::message::Type;
    use zbus::MatchRule;

    static STATE: Mutex<bool> = Mutex::new(false);
    /// Longest wait for the match rule to be installed.
    const READY_TIMEOUT: Duration = Duration::from_secs(2);

    /// Start the listener once; returns when its match rule is installed (or
    /// after `READY_TIMEOUT`). Called from the notification thread.
    pub fn ensure_started() {
        let mut started = lock(&STATE);
        if *started {
            return;
        }
        let (ready_tx, ready_rx) = mpsc::channel::<bool>();
        let spawned = thread::Builder::new()
            .name("kb-notify-actions".into())
            .spawn(move || {
                match subscribe() {
                    Ok((conn, it)) => {
                        let _ = ready_tx.send(true);
                        serve(&conn, it);
                    }
                    Err(e) => {
                        tracing::debug!("notification actions not available: {e}");
                        let _ = ready_tx.send(false);
                    }
                }
                *lock(&STATE) = false;
            });
        if spawned.is_ok() && ready_rx.recv_timeout(READY_TIMEOUT) == Ok(true) {
            *started = true;
        }
    }

    fn subscribe() -> zbus::Result<(Connection, MessageIterator)> {
        let conn = Connection::session()?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.Notifications")?
            .path("/org/freedesktop/Notifications")?
            .build();
        let it = MessageIterator::for_match_rule(rule, &conn, Some(64))?;
        Ok((conn, it))
    }

    /// Only the real owner of the notification name may trigger a button.
    fn from_server(conn: &Connection, sender: Option<&str>) -> bool {
        let Some(sender) = sender else { return false };
        conn.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &("org.freedesktop.Notifications",),
        )
        .ok()
        .and_then(|r| r.body().deserialize::<String>().ok())
        .is_some_and(|owner| owner == sender)
    }

    fn serve(conn: &Connection, it: MessageIterator) {
        for msg in it {
            let Ok(msg) = msg else { continue };
            let header = msg.header();
            let Some(member) = header.member().map(|m| m.as_str().to_string()) else {
                continue;
            };
            if !matches!(member.as_str(), "ActionInvoked" | "NotificationClosed" | "ActivationToken") {
                continue;
            }
            let sender = header.sender().map(|s| s.as_str().to_string());
            if !from_server(conn, sender.as_deref()) {
                continue;
            }
            match member.as_str() {
                "ActionInvoked" => {
                    if let Ok((id, key)) = msg.body().deserialize::<(u32, String)>() {
                        settle_in_flight();
                        let (content, taken) = {
                            let mut reg = lock(registry());
                            (reg.content(id).cloned(), reg.take_action(id, &key))
                        };
                        if let Some((action, token)) = taken {
                            tracing::info!("notification {id}: action {}", action.key());
                            // Own thread: an action may take its time.
                            let _ = thread::Builder::new()
                                .name("kb-notify-action".into())
                                .spawn(move || run_action(action, token, content));
                        }
                    }
                }
                "ActivationToken" => {
                    if let Ok((id, token)) = msg.body().deserialize::<(u32, String)>() {
                        lock(registry()).set_token(id, token);
                    }
                }
                _ => {
                    if let Ok((id, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                        lock(registry()).forget(id);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crossing(threshold: u8, pct: f64, urgency: Urgency) -> Crossing {
        Crossing { threshold, pct, urgency }
    }

    fn rem_low() -> BatteryReminder {
        BatteryReminder {
            level: ReminderLevel::Low,
            mv: 2500,
            threshold_mv: 2506,
        }
    }

    fn rem_crit() -> BatteryReminder {
        BatteryReminder {
            level: ReminderLevel::Critical,
            mv: 2400,
            threshold_mv: 2404,
        }
    }

    #[test]
    fn reminder_states_the_keyboard_threshold_and_the_displayed_percentage() {
        let n = reminder_notification(&rem_low(), Some(35.4), Lang::Fr);
        assert_eq!(n.event, Event::BatteryReminder);
        assert_eq!(n.urgency, Urgency::Normal);
        assert!(
            n.body.contains("2500\u{202f}mV")
                && n.body.contains("seuil Bas du clavier (2506\u{202f}mV)"),
            "{}",
            n.body
        );
        assert!(
            n.body.contains("indication du clavier 35\u{202f}%"),
            "{}",
            n.body
        );
        let c = reminder_notification(&rem_crit(), None, Lang::En);
        assert_eq!(
            (c.urgency, c.icon.as_str()),
            (Urgency::Critical, "battery-empty")
        );
        assert!(
            c.body.contains("Critical threshold (2404\u{202f}mV)") && !c.body.contains('%'),
            "{}",
            c.body
        );
        assert!(c.summary.contains("now"));
        // "Ignore this reminder" silences the Low reminder, never the Critical one.
        REMINDER_IGNORED.store(true, Ordering::SeqCst);
        assert!(reminder_suppressed(ReminderLevel::Low));
        assert!(!reminder_suppressed(ReminderLevel::Critical));
        REMINDER_IGNORED.store(false, Ordering::SeqCst);
        assert!(!reminder_suppressed(ReminderLevel::Low));
    }

    #[test]
    fn crossing_and_replacement_texts() {
        let c = crossing(30, 29.6, Urgency::Normal);
        let (_, b, icon) = crossing_text(&c, AlertBasis::Firmware, Lang::En);
        assert_eq!(
            b,
            "Keyboard indication 30% (below 30%) \u{2014} plan to replace the batteries"
        );
        assert_eq!(
            crossing_text(&c, AlertBasis::Estimate, Lang::En).1,
            "Estimated charge 30% (below 30%) \u{2014} plan to replace the batteries"
        );
        assert_eq!(icon, "battery-caution");
        let c = crossing(5, 4.0, Urgency::Critical);
        assert!(crossing_text(&c, AlertBasis::Estimate, Lang::En)
            .1
            .contains("replace the batteries now"));
        let r = Replacement {
            ts: 0,
            pct_before: Some(3.0),
            pct_after: 100.0,
            voltage_before: None,
            voltage_after: None,
        };
        assert_eq!(
            replaced_text(&r, Lang::En).1,
            "Battery 3% \u{2192} 100%; low-battery alerts re-armed"
        );
        assert!(replaced_text(&r, Lang::Fr).1.contains("alertes de piles faibles"));
    }

    #[test]
    fn every_text_exists_in_both_languages_and_differs() {
        use akm_core::registry::BatteryState as B;
        let c = crossing(15, 14.0, Urgency::Normal);
        let cc = crossing(5, 4.0, Urgency::Critical);
        let ev = [
            LinkEvent::Disconnected { mac: "m".into() },
            LinkEvent::PoweredOff { mac: "m".into() },
            LinkEvent::Reconnected { mac: "m".into(), pct: Some(80.0) },
        ];
        let mut pairs: Vec<(Notification, Notification)> = vec![
            (
                crossing_notification(&c, AlertBasis::Estimate, Lang::En),
                crossing_notification(&c, AlertBasis::Estimate, Lang::Fr),
            ),
            (
                crossing_notification(&cc, AlertBasis::Firmware, Lang::En),
                crossing_notification(&cc, AlertBasis::Firmware, Lang::Fr),
            ),
            (
                reminder_notification(&rem_low(), Some(9.0), Lang::En),
                reminder_notification(&rem_low(), Some(9.0), Lang::Fr),
            ),
            (
                reminder_notification(&rem_crit(), Some(4.0), Lang::En),
                reminder_notification(&rem_crit(), Some(4.0), Lang::Fr),
            ),
            (
                estimate_notification(12.0, 10, Lang::En),
                estimate_notification(12.0, 10, Lang::Fr),
            ),
            (
                removed_notification("Kb", Lang::En),
                removed_notification("Kb", Lang::Fr),
            ),
            (
                firmware_notification("0x0050", "0x0060", Lang::En),
                firmware_notification("0x0050", "0x0060", Lang::Fr),
            ),
        ];
        for s in [B::Low, B::Critical] {
            pairs.push((battery_state_notification(s, Lang::En).unwrap(), battery_state_notification(s, Lang::Fr).unwrap()));
        }
        for e in &ev {
            pairs.push((link_notification(e, Lang::En), link_notification(e, Lang::Fr)));
        }
        for (en, fr) in pairs {
            assert!(!en.summary.is_empty() && !fr.summary.is_empty());
            assert_ne!(en.summary, fr.summary, "{:?}", en.event);
            assert_ne!(en.body, fr.body, "{:?}", en.event);
            assert!(!fr.summary.contains("Apple Keyboard"), "{:?}", fr.summary);
        }
        for a in [Action::Open, Action::Repair, Action::Ignore] {
            assert_ne!(a.label(Lang::En), a.label(Lang::Fr).replace('\u{e9}', "e"));
        }
        assert_eq!(Action::Ignore.label(Lang::Fr), "Ignorer ce rappel");
    }

    #[test]
    fn language_follows_lc_all_then_lc_messages_then_lang() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(Lang::from_vars(env(&[("LANG", "fr_FR.UTF-8")])), Lang::Fr);
        assert_eq!(Lang::from_vars(env(&[("LANG", "en_US.UTF-8")])), Lang::En);
        assert_eq!(Lang::from_vars(env(&[("LC_ALL", "en_GB"), ("LANG", "fr_FR")])), Lang::En);
        assert_eq!(Lang::from_vars(env(&[("LC_ALL", ""), ("LC_MESSAGES", "fr_BE"), ("LANG", "C")])), Lang::Fr);
        assert_eq!(Lang::from_vars(env(&[])), Lang::En);
        assert_eq!(Lang::from_vars(env(&[("LANG", "de_DE")])), Lang::En);
    }

    #[test]
    fn hints_carry_the_kde_identity_and_the_urgency() {
        let n = crossing_notification(&crossing(5, 4.0, Urgency::Critical), AlertBasis::Estimate, Lang::Fr);
        let h = n.hints();
        let s = |k: &str| match &h[k] {
            Value::Str(s) => s.as_str().to_string(),
            v => panic!("{k}: {v:?}"),
        };
        assert_eq!(s("desktop-entry"), "com.agenceapi.AppleKbMonitor");
        assert_eq!(s("x-kde-appname"), "apple-kb-monitor");
        assert_eq!(s("x-kde-eventId"), "BatteryCritical");
        assert_eq!(s("category"), "device");
        assert!(matches!(h["urgency"], Value::U8(2)));
        assert!(!h.contains_key("transient"), "critical alerts stay in the history");
        assert_eq!(n.event.slot(), "battery");
    }

    #[test]
    fn urgency_timeout_and_transience() {
        let crit = crossing_notification(&crossing(5, 4.0, Urgency::Critical), AlertBasis::Estimate, Lang::En);
        assert_eq!(crit.timeout_ms(), 0, "critical = persistent");
        let low = crossing_notification(&crossing(30, 29.0, Urgency::Normal), AlertBasis::Estimate, Lang::En);
        assert_eq!(low.timeout_ms(), 12_000);
        assert!(!low.transient);
        let dis = link_notification(&LinkEvent::Disconnected { mac: "m".into() }, Lang::En);
        assert!(dis.transient && dis.urgency == Urgency::Low);
        assert_eq!(dis.timeout_ms(), 6_000);
        assert!(matches!(dis.hints()["transient"], Value::Bool(true)));
        assert_eq!(dis.event.category(), "device.removed");
        let rec = link_notification(&LinkEvent::Reconnected { mac: "m".into(), pct: None }, Lang::En);
        assert_eq!(rec.event, Event::KeyboardReconnected);
        assert_eq!(rec.event.category(), "device.added");
        assert_eq!(
            rec.event.slot(),
            dis.event.slot(),
            "reconnection replaces the disconnection"
        );
        assert_eq!(
            reminder_notification(&rem_low(), Some(10.0), Lang::En).timeout_ms(),
            15_000
        );
        assert_eq!(
            reminder_notification(&rem_crit(), None, Lang::En).timeout_ms(),
            0,
            "critical: persistent"
        );
    }

    #[test]
    fn events_map_to_the_right_buttons() {
        let list = |n: Notification| n.action_list();
        let rep = notice_notification(Event::RepairNeeded, "s", "b", Urgency::Critical);
        let l = list(rep);
        let keys: Vec<&str> = l.iter().step_by(2).map(|s| s.as_str()).collect();
        assert_eq!(keys, ["default", "repair", "open"]);
        let rem = list(reminder_notification(&rem_low(), Some(9.0), Lang::Fr));
        assert_eq!(rem[1], "Ouvrir");
        assert!(
            rem.contains(&"ignore".to_string()) && rem.contains(&"Ignorer ce rappel".to_string())
        );
        // #110: "Remind me tomorrow" between "Open" and "Ignore".
        let keys: Vec<&str> = rem.iter().step_by(2).map(|s| s.as_str()).collect();
        assert_eq!(keys, ["default", "open", "remind", "ignore"]);
        assert!(rem.contains(&"Me rappeler demain".to_string()));
        assert!(
            generic_notification("s", "b", "dialog-error", Urgency::Critical, false)
                .action_list()
                .is_empty()
        );
        assert_eq!(
            list(crossing_notification(
                &crossing(30, 29.0, Urgency::Normal),
                AlertBasis::Estimate,
                Lang::En
            )),
            [
                "default",
                "Open",
                "open",
                "Open",
                "remind",
                "Remind me tomorrow"
            ]
        );
        // What must be dealt with now offers no "tomorrow".
        for urgent in [
            Event::BatteryCritical,
            Event::RepairNeeded,
            Event::KeyboardAlert,
        ] {
            assert!(
                !urgent.actions().contains(&Action::RemindTomorrow),
                "{urgent:?}"
            );
        }
        assert_eq!(Action::from_key("remind"), Some(Action::RemindTomorrow));
    }

    /// A notification put aside comes back identical (texts, buttons, hints).
    #[test]
    fn a_stored_notification_comes_back_identical() {
        for n in [
            crossing_notification(
                &crossing(30, 29.0, Urgency::Normal),
                AlertBasis::Estimate,
                Lang::Fr,
            ),
            reminder_notification(&rem_crit(), Some(4.0), Lang::En),
            link_notification(&LinkEvent::Disconnected { mac: "m".into() }, Lang::Fr),
            removed_notification("Clavier <b>de</b> alice", Lang::Fr),
        ] {
            let back = Notification::from_stored(&n.to_stored()).unwrap();
            assert_eq!(back, n, "body escaped once, not twice");
        }
        let mut s = low_stored();
        s.event = "NoSuchEvent".into();
        assert!(Notification::from_stored(&s).is_none());
        for e in Event::ALL {
            assert_eq!(Event::from_id(e.id()), Some(e));
        }
    }

    fn low_stored() -> akm_core::deferred::Stored {
        crossing_notification(
            &crossing(30, 29.0, Urgency::Normal),
            AlertBasis::Estimate,
            Lang::En,
        )
        .to_stored()
    }

    #[test]
    fn event_ids_are_unique() {
        let mut ids: Vec<&str> = Event::ALL.iter().map(|e| e.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), Event::ALL.len());
    }

    /// The notifyrc declares every event the daemon sends, with a French
    /// name and comment, and its [Global] section points to the .desktop.
    #[test]
    fn notifyrc_declares_every_event_in_french_and_english() {
        let rc = include_str!("../../../data/apple-kb-monitor.notifyrc");
        assert!(rc.contains("[Global]") && rc.contains("DesktopEntry=com.agenceapi.AppleKbMonitor"));
        assert!(rc.contains("Name[fr]=") && rc.contains("IconName="));
        let mut sections: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut cur = "";
        for line in rc.lines() {
            if let Some(s) = line.strip_prefix("[Event/").and_then(|s| s.strip_suffix(']')) {
                cur = s;
                sections.entry(cur).or_default();
            } else if line.starts_with('[') {
                cur = "";
            } else if !cur.is_empty() {
                sections.get_mut(cur).unwrap().push(line);
            }
        }
        for e in Event::ALL {
            let body = sections.get(e.id()).unwrap_or_else(|| panic!("no [Event/{}]", e.id()));
            for key in ["Name=", "Name[fr]=", "Comment=", "Comment[fr]=", "Action="] {
                assert!(body.iter().any(|l| l.starts_with(key)), "{}: {key}", e.id());
            }
        }
        assert_eq!(sections.len(), Event::ALL.len(), "events in the notifyrc but not in the daemon");
    }

    #[test]
    fn desktop_entry_matches_the_identity() {
        let d = include_str!("../../../com.agenceapi.AppleKbMonitor.desktop");
        assert!(d.contains("DBusActivatable=true") && d.contains("StartupNotify=true"));
        assert!(d.contains("X-KDE-StartupNotify=true"));
        assert!(d.contains(&format!("StartupWMClass={DESKTOP_ENTRY}")));
        let rc = include_str!("../../../data/apple-kb-monitor.notifyrc");
        assert!(rc.contains(&format!("DesktopEntry={DESKTOP_ENTRY}")));
        assert!(d.contains("Name[fr]="));
    }

    #[test]
    fn registry_replaces_per_slot_and_checks_the_button() {
        let mut r = Registry::default();
        assert_eq!(r.replace_id("battery"), 0);
        r.record("battery", 7, Event::BatteryLow);
        assert_eq!(r.replace_id("battery"), 7, "next battery alert replaces #7");
        assert_eq!(r.replace_id("link"), 0, "other slots are independent");
        // The server answered another id (the old one was closed meanwhile).
        r.record("battery", 9, Event::BatteryCritical);
        assert_eq!(r.replace_id("battery"), 9);
        assert!(r.take_action(7, "open").is_none(), "7 is forgotten");
        // A button the notification does not offer is refused.
        assert!(r.take_action(9, "repair").is_none());
        assert!(r.take_action(9, "bogus").is_none());
        r.set_token(9, "tok".into());
        assert_eq!(r.take_action(9, "default"), Some((Action::Open, Some("tok".into()))));
        assert!(r.take_action(9, "open").is_none(), "one action per notification");
        assert_eq!(r.replace_id("battery"), 0);
        // Closed: forgotten.
        r.record("link", 3, Event::KeyboardDisconnected);
        r.forget(3);
        assert_eq!(r.replace_id("link"), 0);
        // Bounded.
        for i in 0..200u32 {
            r.record("error", 1000 + i, Event::Error);
        }
        assert!(r.pending.len() <= MAX_PENDING && r.order.len() <= MAX_PENDING);
    }

    #[test]
    fn mute_server_never_blocks_the_caller() {
        use std::time::Instant;
        // #162: jobs that never return must not delay submit(), nor the next ones.
        let n = Notifier::new(Duration::from_millis(100), 2);
        let t = Instant::now();
        for _ in 0..8 {
            n.submit(|| thread::sleep(Duration::from_secs(30)));
        }
        assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
        // Once the 2 allowed blocked calls are stuck, the rest is dropped,
        // never run, never waited for.
        let (tx, rx) = mpsc::channel();
        thread::sleep(Duration::from_millis(500));
        n.submit(move || {
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    fn healthy_server_gets_every_job_in_order() {
        let n = Notifier::new(Duration::from_secs(1), 4);
        let (tx, rx) = mpsc::channel();
        for i in 0..5 {
            let tx = tx.clone();
            n.submit(move || {
                let _ = tx.send(i);
            });
        }
        let got: Vec<i32> = (0..5)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn low_battery_text_rounds() {
        let (s, b) = low_battery_text(12.4, Lang::En);
        assert!(s.contains("Low Battery"));
        assert_eq!(b, "Battery at 12% \u{2014} charge soon");
        assert!(low_battery_text(12.4, Lang::Fr).1.contains("12\u{202f}%"));
    }

    #[test]
    fn keyboard_driven_alert_texts() {
        use akm_core::registry::BatteryState as B;
        let (sum, b, icon, u) = battery_state_text(B::Low, Lang::En).unwrap();
        assert!(sum.contains("battery low (keyboard alert)"));
        assert!(b.contains("low battery") && icon == "battery-caution" && u == Urgency::Normal);
        let (sum, b, icon, u) = battery_state_text(B::Critical, Lang::En).unwrap();
        assert!(sum.contains("battery critical (keyboard alert)"));
        assert!(b.contains("replace the batteries now") && icon == "battery-empty" && u == Urgency::Critical);
        assert!(battery_state_text(B::Normal, Lang::En).is_none() && battery_state_text(B::Invalid(9), Lang::En).is_none());
        assert_eq!(battery_state_notification(B::Critical, Lang::En).unwrap().event, Event::KeyboardAlert);
    }
}
