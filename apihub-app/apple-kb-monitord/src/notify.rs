//! Desktop notifications through zbus directly (one zbus version in the tree), in `KNotification`'s
//! dialect so Plasma treats the daemon as an application.

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

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const QUEUE: usize = 16;
const MAX_BLOCKED: usize = 4;

type Job = Box<dyn FnOnce() + Send>;

/// Hands notifications to a dedicated thread.
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
                    let started =
                        thread::Builder::new()
                            .name("kb-notify-call".into())
                            .spawn(move || {
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

    /// Queue a job; never blocks.
    pub fn submit(&self, job: impl FnOnce() + Send + 'static) -> bool {
        if let Ok(()) = self.tx.try_send(Box::new(job)) {
            true
        } else {
            tracing::warn!("notification dropped (queue full or sender gone)");
            false
        }
    }
}

fn notifier() -> &'static Notifier {
    static N: OnceLock<Notifier> = OnceLock::new();
    N.get_or_init(|| Notifier::new(CALL_TIMEOUT, MAX_BLOCKED))
}

/// Name of the `KNotification` event file, sent as the `x-kde-appname` hint.
pub const KDE_APPNAME: &str = "apple-kb-monitor";
/// `app_name` argument of `Notify`: the display name of the application.
pub const APP_NAME: &str = "Apple Keyboard Monitor";

/// `KNotification` events: each one is a `[Event/<id>]` of the notifyrc.
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
    /// More than 3 disconnections within an hour.
    LinkUnstable,
    /// "Forget the keyboard?": the confirmation asked by the tray.
    ForgetConfirm,
    /// The Fn mode remembered for the keyboard that reconnected differs from the one in effect.
    SettingsReapply,
    BatteryEstimate,
    FirmwareUpdate,
    BatteryReminder,
    BatteryReplaced,
    /// "Batteries changed too often".
    BatteryAdvice,
    Error,
}

impl Event {
    pub const ALL: [Event; 18] = [
        Event::BatteryLow,
        Event::BatteryCritical,
        Event::KeyboardAlert,
        Event::KeyboardDisconnected,
        Event::KeyboardReconnected,
        Event::KeyboardOff,
        Event::KeyboardUnreachable,
        Event::RepairNeeded,
        Event::KeyboardRemoved,
        Event::LinkUnstable,
        Event::ForgetConfirm,
        Event::SettingsReapply,
        Event::BatteryEstimate,
        Event::FirmwareUpdate,
        Event::BatteryReminder,
        Event::BatteryReplaced,
        Event::BatteryAdvice,
        Event::Error,
    ];

    /// Value of `x-kde-eventId` = name of the `[Event/<id>]` section.
    #[must_use]
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
            Event::LinkUnstable => "LinkUnstable",
            Event::ForgetConfirm => "ForgetConfirm",
            Event::SettingsReapply => "SettingsReapply",
            Event::BatteryEstimate => "BatteryEstimate",
            Event::FirmwareUpdate => "FirmwareUpdate",
            Event::BatteryReminder => "BatteryReminder",
            Event::BatteryReplaced => "BatteryReplaced",
            Event::BatteryAdvice => "BatteryAdvice",
            Event::Error => "Error",
        }
    }

    /// Notifications of one slot replace each other (`replaces_id`).
    #[must_use]
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
            Event::LinkUnstable => "link-quality",
            Event::ForgetConfirm => "forget",
            Event::SettingsReapply => "settings",
            Event::FirmwareUpdate => "firmware",
            // Its own slot: "new batteries", sent just before, stays visible.
            Event::BatteryAdvice => "advice",
            Event::Error => "error",
        }
    }

    /// freedesktop.org `category` hint.
    #[must_use]
    pub fn category(self) -> &'static str {
        match self {
            Event::KeyboardReconnected => "device.added",
            Event::KeyboardDisconnected | Event::KeyboardOff | Event::KeyboardRemoved => {
                "device.removed"
            }
            Event::LinkUnstable
            | Event::KeyboardUnreachable
            | Event::RepairNeeded
            | Event::Error => "device.error",
            _ => "device",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Event> {
        Event::ALL.into_iter().find(|e| e.id() == id)
    }

    /// Buttons of the event (the body click, `default`, always does the first one if it is `Open`).
    #[must_use]
    pub fn actions(self) -> &'static [Action] {
        match self {
            Event::RepairNeeded | Event::KeyboardRemoved => &[Action::Repair, Action::Open],
            // One button, and no `default`: a click on the body forgets nothing.
            Event::ForgetConfirm => &[Action::Forget],
            // No `default` either: an authentication follows the button.
            Event::SettingsReapply => &[Action::ApplySettings],
            Event::BatteryReminder => &[Action::Open, Action::RemindTomorrow, Action::Ignore],
            // Nothing urgent: may be shown again tomorrow.
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
    /// Open the widget's popup (`Tray.OpenPanel` → signal `PanelRequested`).
    Open,
    /// `akmctl repair` in a terminal.
    Repair,
    /// Stop the battery reminder.
    Ignore,
    /// Show this notification again in 24 hours.
    RemindTomorrow,
    /// Confirm "Forget this keyboard" asked from the tray.
    Forget,
    /// Put back the Fn mode remembered for the keyboard.
    ApplySettings,
}

impl Action {
    /// Identifier sent in the `actions` array.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Action::Open => "open",
            Action::Repair => "repair",
            Action::Ignore => "ignore",
            Action::RemindTomorrow => "remind",
            Action::Forget => "forget",
            Action::ApplySettings => "apply",
        }
    }

    #[must_use]
    pub fn label(self) -> String {
        match self {
            Action::Open => tr!("Open"),
            Action::Repair => tr!("Repair\u{2026}"),
            Action::Ignore => tr!("Ignore this reminder"),
            Action::RemindTomorrow => tr!("Remind me tomorrow"),
            Action::Forget => tr!("Forget"),
            Action::ApplySettings => tr!("Apply"),
        }
    }

    /// `ActionInvoked` key to action; `default` (click on the body) opens.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Action> {
        match key {
            "default" | "open" => Some(Action::Open),
            "repair" => Some(Action::Repair),
            "ignore" => Some(Action::Ignore),
            "remind" => Some(Action::RemindTomorrow),
            "forget" => Some(Action::Forget),
            "apply" => Some(Action::ApplySettings),
            _ => None,
        }
    }
}

/// Everything of one `Notify` call but the id to replace.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub event: Event,
    pub summary: String,
    pub body: String,
    pub icon: String,
    pub urgency: Urgency,
    pub transient: bool,
}

/// Escapes the body markup of the freedesktop notification spec (`&`, `<`, `>`, `"`).
#[must_use]
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
    fn new(event: Event, summary: String, body: &str, icon: &str, urgency: Urgency) -> Self {
        Self {
            event,
            summary,
            body: escape_markup(body),
            icon: icon.to_string(),
            urgency,
            // Low-urgency notifications are not kept in the history of the server.
            transient: urgency == Urgency::Low,
        }
    }

    /// What is kept of a notification shown later: the texts as shown, nothing else.
    #[must_use]
    pub fn to_stored(&self) -> akm_core::deferred::Stored {
        akm_core::deferred::Stored {
            event: self.event.id().to_string(),
            summary: self.summary.clone(),
            body: self.body.clone(),
            icon: self.icon.clone(),
            urgency: self.urgency.hint(),
            transient: self.transient,
        }
    }

    /// Back from [`Self::to_stored`] (the body is already escaped).
    #[must_use]
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
        })
    }

    /// `expire_timeout` in ms: critical = 0, otherwise a delay long enough to read.
    #[must_use]
    pub fn timeout_ms(&self) -> i32 {
        match (self.urgency, self.event) {
            // As long as the confirmation is accepted.
            (_, Event::ForgetConfirm) => {
                i32::try_from(crate::forget::CONFIRM_WINDOW.as_millis()).unwrap_or(i32::MAX)
            }
            (Urgency::Critical, _) => 0,
            (_, Event::BatteryReminder) => 15_000,
            (Urgency::Low, _) => 6_000,
            (Urgency::Normal, _) => 12_000,
        }
    }

    /// The `actions` array of `Notify`: `default` (click) then the buttons.
    #[must_use]
    pub fn action_list(&self) -> Vec<String> {
        let acts = self.event.actions();
        let mut v = Vec::new();
        if acts.contains(&Action::Open) {
            v.push("default".to_string());
            v.push(Action::Open.label());
        }
        for a in acts {
            v.push(a.key().to_string());
            v.push(a.label());
        }
        v
    }

    /// The hints Plasma reads: `urgency`, `category`, the KDE pair `x-kde-appname` /
    /// `x-kde-eventId`, `transient` (no `desktop-entry`: no `.desktop` file is shipped).
    #[must_use]
    pub fn hints(&self) -> HashMap<&'static str, Value<'static>> {
        let mut h: HashMap<&'static str, Value<'static>> = HashMap::from([
            ("urgency", Value::U8(self.urgency.hint())),
            ("category", Value::from(self.event.category().to_string())),
            ("x-kde-appname", Value::from(KDE_APPNAME.to_string())),
            ("x-kde-eventId", Value::from(self.event.id().to_string())),
        ]);
        if self.transient {
            h.insert("transient", Value::Bool(true));
        }
        h
    }
}

fn pct_s(p: f64) -> String {
    tr!("{pct}%", pct = format!("{p:.0}"))
}

#[must_use]
pub fn low_battery_text(pct: f64) -> (String, String) {
    (
        tr!("Apple Keyboard \u{2014} Low Battery"),
        tr!("Battery at {pct} \u{2014} charge soon", pct = pct_s(pct)),
    )
}

/// Text and icon of a threshold alert.
#[must_use]
pub fn crossing_text(c: &Crossing, basis: AlertBasis) -> (String, String, &'static str) {
    let (summary, _) = low_battery_text(c.pct);
    let what = match basis {
        AlertBasis::Estimate => tr!("Estimated charge"),
        AlertBasis::Firmware => tr!("Keyboard indication"),
    };
    let critical = c.urgency == Urgency::Critical;
    let body = if critical {
        tr!(
            "{what} {pct} \u{2014} replace the batteries now",
            what = what,
            pct = pct_s(c.pct)
        )
    } else {
        tr!(
            "{what} {pct} (below {threshold}%) \u{2014} plan to replace the batteries",
            what = what,
            pct = pct_s(c.pct),
            threshold = c.threshold
        )
    };
    let icon = if critical {
        "battery-empty"
    } else {
        "battery-caution"
    };
    (summary, body, icon)
}

#[must_use]
pub fn crossing_notification(c: &Crossing, basis: AlertBasis) -> Notification {
    let (s, b, icon) = crossing_text(c, basis);
    let event = if c.urgency == Urgency::Critical {
        Event::BatteryCritical
    } else {
        Event::BatteryLow
    };
    Notification::new(event, s, &b, icon, c.urgency)
}

pub fn battery_crossing(c: &Crossing, basis: AlertBasis) {
    deliver(crossing_notification(c, basis));
}

/// Text, icon and urgency of a keyboard-driven battery alert (`0x30`).
#[must_use]
pub fn battery_state_text(
    s: akm_core::registry::BatteryState,
) -> Option<(String, String, &'static str, Urgency)> {
    use akm_core::registry::BatteryState as B;
    match s {
        B::Low => Some((
            tr!("Apple Keyboard \u{2014} battery low (keyboard alert)"),
            tr!("The keyboard itself reports a low battery \u{2014} plan to replace the batteries"),
            "battery-caution",
            Urgency::Normal,
        )),
        B::Critical => Some((
            tr!("Apple Keyboard \u{2014} battery critical (keyboard alert)"),
            tr!("The keyboard itself reports a critically low battery \u{2014} replace the batteries now"),
            "battery-empty",
            Urgency::Critical,
        )),
        B::Normal | B::Invalid(_) => None,
    }
}

#[must_use]
pub fn battery_state_notification(s: akm_core::registry::BatteryState) -> Option<Notification> {
    let (sum, body, icon, urg) = battery_state_text(s)?;
    Some(Notification::new(
        Event::KeyboardAlert,
        sum,
        &body,
        icon,
        urg,
    ))
}

/// Keyboard-driven alert (Input `0x30`).
pub fn battery_state(s: akm_core::registry::BatteryState) {
    if let Some(n) = battery_state_notification(s) {
        deliver(n);
    }
}

/// Text of a connection change.
#[must_use]
pub fn link_text(ev: &LinkEvent) -> (String, String) {
    link::text(ev)
}

/// Disconnected / reconnected / switched off: low urgency, transient.
#[must_use]
pub fn link_notification(ev: &LinkEvent) -> Notification {
    let (s, b) = link_text(ev);
    let (event, icon) = match ev {
        LinkEvent::Disconnected { .. } => {
            (Event::KeyboardDisconnected, "input-keyboard-virtual-off")
        }
        LinkEvent::PoweredOff { .. } => (Event::KeyboardOff, "input-keyboard-virtual-off"),
        LinkEvent::Reconnected { .. } => (Event::KeyboardReconnected, "input-keyboard"),
    };
    Notification::new(event, s, &b, icon, Urgency::Low)
}

pub fn link(ev: &LinkEvent) {
    deliver(link_notification(ev));
}

/// Text of the "new batteries" notification.
pub fn replaced_text(r: &Replacement) -> (String, String) {
    let before = r.pct_before.map_or("?".to_string(), pct_s);
    (
        tr!("Apple Keyboard \u{2014} new batteries"),
        tr!(
            "Battery {before} \u{2192} {after}; low-battery alerts re-armed",
            before = before,
            after = pct_s(r.pct_after)
        ),
    )
}

#[must_use]
pub fn replaced_notification(r: &Replacement) -> Notification {
    let (s, b) = replaced_text(r);
    Notification::new(
        Event::BatteryReplaced,
        s,
        &b,
        "battery-full",
        Urgency::Normal,
    )
}

/// "Batteries changed too often": the last two sets lasted less than 30 days each.
#[must_use]
pub fn advice_notification(a: &akm_core::advice::ShortLife) -> Notification {
    Notification::new(
        Event::BatteryAdvice,
        tr!("Apple Keyboard \u{2014} batteries changed too often"),
        &format!(
            "{}. {}",
            akm_core::advice::line(a),
            akm_core::advice::recommendation()
        ),
        "battery-caution",
        Urgency::Normal,
    )
}

pub fn battery_advice(a: &akm_core::advice::ShortLife) {
    deliver(advice_notification(a));
}

pub fn battery_replaced(r: &Replacement) {
    // New batteries end any snoozed reminder, and what waited to be shown again about the old ones.
    REMINDER_IGNORED.store(false, Ordering::SeqCst);
    crate::notify_policy::cancel_slot(Event::BatteryReplaced.slot());
    deliver(replaced_notification(r));
}

/// "Change the batteries" reminder.
#[must_use]
pub fn reminder_notification(r: &BatteryReminder, pct: Option<f64>) -> Notification {
    let critical = r.level == ReminderLevel::Critical;
    let (mv, t) = (r.mv, r.threshold_mv);
    let pct = pct.filter(|p| p.is_finite());
    let level = if critical {
        tr!("Critical")
    } else {
        tr!("Low")
    };
    let indication = pct.map_or(String::new(), |p| {
        tr!("; keyboard indication {pct}", pct = pct_s(p))
    });
    let todo = if critical {
        tr!("change the batteries now")
    } else {
        tr!("plan to change the batteries")
    };
    let body = tr!(
        "Voltage {mv}\u{202f}mV, under the keyboard's {level} threshold ({t}\u{202f}mV){indication} \u{2014} {todo}",
        mv = mv,
        level = level,
        t = t,
        indication = indication,
        todo = todo
    );
    let summary = if critical {
        &tr!("Apple Keyboard \u{2014} change the batteries now")
    } else {
        &tr!("Apple Keyboard \u{2014} time to change the batteries")
    };
    let (icon, urgency) = if critical {
        ("battery-empty", Urgency::Critical)
    } else {
        ("battery-caution", Urgency::Normal)
    };
    Notification::new(Event::BatteryReminder, summary.into(), &body, icon, urgency)
}

/// The single reminder that replaces our percentage alerts when KDE `PowerDevil` already warns about
/// this keyboard.
#[must_use]
pub fn estimate_notification(pct: f64, low_level: u8) -> Notification {
    let body = tr!(
        "Estimate from your batteries: about {pct} left. KDE warns separately at {low}% of the keyboard's own indication.",
        pct = pct_s(pct),
        low = low_level
    );
    Notification::new(
        Event::BatteryEstimate,
        tr!("Apple Keyboard \u{2014} batteries getting low"),
        &body,
        "battery-caution",
        Urgency::Normal,
    )
}

pub fn battery_estimate(pct: f64, low_level: u8) {
    deliver(estimate_notification(pct, low_level));
}

/// The keyboard `name` reconnected and the Fn mode remembered for it (`want`) is not the one in
/// effect (`live`).
#[must_use]
pub fn reapply_notification(name: &str, want: i32, live: i32) -> Notification {
    let word = |m: i32| akm_core::hid_params::fn_mode_label(m).unwrap_or_else(|| m.to_string());
    let body = tr!("\u{201c}{name}\u{201d} reconnected. Fn mode remembered for it: {want}; in effect: {live}. \
             This setting (hid_apple) is common to every Apple keyboard of this computer; \
             applying it asks for an authentication.", want = word(want), live = word(live), name = name);
    Notification::new(
        Event::SettingsReapply,
        tr!("Apple Keyboard \u{2014} remembered Fn mode"),
        &body,
        "input-keyboard",
        Urgency::Normal,
    )
}

#[must_use]
pub fn forget_confirm_notification(name: &str) -> Notification {
    let body = tr!(
        "\u{201c}{name}\u{201d} will be removed from this computer. To use it again you will have \
             to switch it off and on and pair it again. Nothing is written to the keyboard. \
             Press \u{201c}Forget\u{201d} within one minute to confirm.",
        name = name
    );
    Notification::new(
        Event::ForgetConfirm,
        tr!("Forget the keyboard?"),
        &body,
        "dialog-warning",
        Urgency::Critical,
    )
}

/// The link keeps dropping: `count` disconnections within the last hour.
#[must_use]
pub fn unstable_notification(name: &str, count: usize) -> Notification {
    let body = trn!("\u{201c}{name}\u{201d} disconnected {count} time within the last hour. Check the \
             batteries, the distance and USB 3 devices near the Bluetooth adapter; details: akmctl doctor.",
        "\u{201c}{name}\u{201d} disconnected {count} times within the last hour. Check the \
             batteries, the distance and USB 3 devices near the Bluetooth adapter; details: akmctl doctor.", count, name = name, count = count);
    Notification::new(
        Event::LinkUnstable,
        tr!("Keyboard: unstable link"),
        &body,
        "network-wireless-disconnected",
        Urgency::Normal,
    )
}

pub fn link_unstable(name: &str, count: usize) {
    deliver(unstable_notification(name, count));
}

/// The keyboard was removed from this computer from outside (Plasma "Forget", `bluetoothctl
/// remove`): what happened and the next step.
#[must_use]
pub fn removed_notification(name: &str) -> Notification {
    let body = tr!("\u{201c}{name}\u{201d} was removed from this computer: switch it off and on to pair it again \
             (button Repair, or \u{201c}akmctl repair\u{201d}).", name = name);
    Notification::new(
        Event::KeyboardRemoved,
        tr!("Keyboard removed from this computer"),
        &body,
        "input-keyboard-virtual-off",
        Urgency::Normal,
    )
}

pub fn keyboard_removed(name: &str) {
    deliver(removed_notification(name));
}

#[cfg(any(test, feature = "testbus"))]
/// Send the reminder unless the user ignored it (until new batteries).
#[must_use]
pub fn battery_reminder(r: &BatteryReminder, pct: Option<f64>) -> bool {
    if reminder_suppressed(r.level) {
        return false;
    }
    deliver(reminder_notification(r, pct));
    true
}

/// "Ignore this reminder" applies to the Low reminder only.
pub fn reminder_suppressed(level: ReminderLevel) -> bool {
    level == ReminderLevel::Low && REMINDER_IGNORED.load(Ordering::SeqCst)
}

/// A newer firmware than the keyboard's is known.
#[must_use]
pub fn firmware_notification(current: &str, latest: &str) -> Notification {
    let body = tr!(
        "Firmware {current}; latest version known: {latest}",
        current = current,
        latest = latest
    );
    Notification::new(
        Event::FirmwareUpdate,
        tr!("Apple Keyboard \u{2014} firmware update known"),
        &body,
        "software-update-available",
        Urgency::Normal,
    )
}

/// A notice of the link keeper, tied to its KDE event.
#[must_use]
pub fn notice_notification(
    event: Event,
    summary: &str,
    body: &str,
    urgency: Urgency,
) -> Notification {
    Notification::new(event, summary.to_string(), body, "input-keyboard", urgency)
}

pub fn notice(event: Event, summary: &str, body: &str, urgency: Urgency) {
    deliver(notice_notification(event, summary, body, urgency));
}

/// Anything else (errors): the `Error` event, no button.
#[must_use]
pub fn generic_notification(
    summary: &str,
    body: &str,
    icon: &str,
    urgency: Urgency,
    transient: bool,
) -> Notification {
    let mut n = Notification::new(Event::Error, summary.to_string(), body, icon, urgency);
    n.transient = transient;
    n
}

/// Send a critical notification on the session bus; errors are logged, never fatal.
pub fn send(summary: &str, body: &str, icon: &str) {
    send_with(summary, body, icon, Urgency::Critical, false);
}

/// Send with an urgency; `transient` notifications are not kept in the history of the notification
/// server (connection changes).
pub fn send_with(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) {
    deliver(generic_notification(
        summary, body, icon, urgency, transient,
    ));
}

/// Queue a notification for the notification thread; never blocks.
pub fn deliver(n: Notification) {
    let Some(n) = crate::notify_policy::admit(n) else {
        return;
    };
    notifier().submit(move || {
        deliver_blocking(&n);
    });
}

/// Show what is due now: notifications held by the quiet hours that just ended, and "Remind me
/// tomorrow" reminders.
pub fn tick() {
    for n in crate::notify_policy::take_due() {
        deliver(n);
    }
}

#[cfg(any(test, feature = "testbus"))]
/// The blocking D-Bus call, for a generic notification (notification thread only).
pub fn send_blocking(summary: &str, body: &str, icon: &str, urgency: Urgency, transient: bool) {
    deliver_blocking(&generic_notification(
        summary, body, icon, urgency, transient,
    ));
}

/// The blocking `Notify` call (notification thread only).
pub fn deliver_blocking(n: &Notification) -> Option<u32> {
    if !n.event.actions().is_empty() {
        listener::ensure_started();
    }
    let replaces = lock(registry()).replace_id(n.event.slot());
    // The server may hand the click on a button back before its reply to `Notify` is recorded
    // below: the listener waits for calls in flight.
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let res = with_session(|conn| {
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
    });
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

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Longest wait of the button listener for such a call.
const IN_FLIGHT_WAIT: Duration = Duration::from_millis(500);

fn settle_in_flight() {
    let end = std::time::Instant::now() + IN_FLIGHT_WAIT;
    while IN_FLIGHT.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < end {
        thread::sleep(Duration::from_millis(5));
    }
}

/// Runs `f` on one shared session-bus connection, opened on first use and again after a bus error.
fn with_session<T>(
    f: impl FnOnce(&zbus::blocking::Connection) -> zbus::Result<T>,
) -> zbus::Result<T> {
    static SESSION: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);
    let conn = {
        let mut g = lock(&SESSION);
        match g.as_ref() {
            Some(c) => c.clone(),
            None => g.insert(zbus::blocking::Connection::session()?).clone(),
        }
    };
    let r = f(&conn);
    if matches!(&r, Err(e) if !matches!(e, zbus::Error::MethodError(..))) {
        *lock(&SESSION) = None;
    }
    r
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

const MAX_PENDING: usize = 64;

static REMINDER_IGNORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which server id each slot shows, and which ids carry buttons.
#[derive(Debug, Default)]
pub struct Registry {
    slots: HashMap<&'static str, u32>,
    pending: HashMap<u32, Event>,
    order: VecDeque<u32>,
    tokens: HashMap<u32, String>,
    content: HashMap<u32, Notification>,
}

impl Registry {
    /// `replaces_id` for the next notification of the slot (0 = new).
    #[must_use]
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
    #[must_use]
    pub fn content(&self, id: u32) -> Option<&Notification> {
        self.content.get(&id)
    }

    pub fn set_token(&mut self, id: u32, token: String) {
        if self.pending.contains_key(&id) {
            self.tokens.insert(id, token);
        }
    }

    /// An action was invoked on `id`.
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

#[cfg(any(test, feature = "testbus"))]
type Handler = Arc<dyn Fn(Action, Option<String>) + Send + Sync>;

#[cfg(any(test, feature = "testbus"))]
fn handler_slot() -> &'static Mutex<Option<Handler>> {
    static H: OnceLock<Mutex<Option<Handler>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

/// Replace what the buttons do (tests).
#[cfg(any(test, feature = "testbus"))]
pub fn set_action_handler(h: impl Fn(Action, Option<String>) + Send + Sync + 'static) {
    *lock(handler_slot()) = Some(Arc::new(h));
}

fn run_action(action: Action, token: Option<&str>, content: Option<&Notification>) {
    // "Remind me tomorrow" is the daemon's own doing: no handler replaces it.
    if action == Action::RemindTomorrow {
        if let Some(n) = content.as_ref() {
            crate::notify_policy::remind(n);
        } else {
            tracing::warn!("notification: nothing kept to remind of");
        }
    }
    #[cfg(any(test, feature = "testbus"))]
    if let Some(h) = lock(handler_slot()).clone() {
        h(action, token.map(str::to_owned));
        return;
    }
    match action {
        Action::Open => open_panel(),
        Action::Repair => crate::repair::launch_repair(token),
        Action::Ignore => {
            REMINDER_IGNORED.store(true, Ordering::SeqCst);
            crate::notify_policy::cancel_slot(Event::BatteryReminder.slot());
            tracing::info!("notification: battery reminder ignored");
        }
        Action::RemindTomorrow => {}
        Action::Forget => {
            crate::forget::confirmed();
        }
        Action::ApplySettings => crate::reapply::apply_pending(),
    }
}

/// "Open": the daemon asks the Plasma widget to open its popup (`Tray.OpenPanel`).
fn open_panel() {
    let r = with_session(|c| {
        c.call_method(
            Some(crate::service::bus_name()),
            "/com/agenceapi/AppleKbMonitor1/Tray",
            Some("com.agenceapi.AppleKbMonitor1.Tray"),
            "OpenPanel",
            &(),
        )
    });
    if let Err(e) = r {
        tracing::warn!("notification: widget popup not opened: {e}");
    }
}

mod listener {
    use super::{lock, mpsc, registry, run_action, settle_in_flight, thread, Duration, Mutex};
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::message::Type;
    use zbus::MatchRule;

    use crate::origin::Origin;

    static STATE: Mutex<bool> = Mutex::new(false);
    const READY_TIMEOUT: Duration = Duration::from_secs(2);

    /// Start the listener once; returns when its match rule is installed.
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
                    Ok((it, origin)) => {
                        let _ = ready_tx.send(true);
                        serve(it, origin);
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

    // The rule filters broadcasts only: `serve` also drops what another peer sends to us directly.
    fn subscribe() -> zbus::Result<(MessageIterator, Origin)> {
        let conn = Connection::session()?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.Notifications")?
            .interface("org.freedesktop.Notifications")?
            .path("/org/freedesktop/Notifications")?
            .build();
        let it = MessageIterator::for_match_rule(rule, &conn, Some(64))?;
        Ok((it, Origin::new(&conn, &["org.freedesktop.Notifications"])))
    }

    fn serve(it: MessageIterator, mut origin: Origin) {
        for msg in it {
            let Ok(msg) = msg else { continue };
            if !origin.accept(&msg) {
                continue;
            }
            let header = msg.header();
            let Some(member) = header.member().map(|m| m.as_str().to_string()) else {
                continue;
            };
            if !matches!(
                member.as_str(),
                "ActionInvoked" | "NotificationClosed" | "ActivationToken"
            ) {
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
                                .spawn(move || {
                                    run_action(action, token.as_deref(), content.as_ref());
                                });
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
        Crossing {
            threshold,
            pct,
            urgency,
        }
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
        let n = reminder_notification(&rem_low(), Some(35.4));
        assert_eq!(n.event, Event::BatteryReminder);
        assert_eq!(n.urgency, Urgency::Normal);
        assert!(
            n.body.contains("2500\u{202f}mV")
                && n.body.contains("keyboard's Low threshold (2506\u{202f}mV)"),
            "{}",
            n.body
        );
        assert!(n.body.contains("keyboard indication 35%"), "{}", n.body);
        let c = reminder_notification(&rem_crit(), None);
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
        let (_, b, icon) = crossing_text(&c, AlertBasis::Firmware);
        assert_eq!(
            b,
            "Keyboard indication 30% (below 30%) \u{2014} plan to replace the batteries"
        );
        assert_eq!(
            crossing_text(&c, AlertBasis::Estimate).1,
            "Estimated charge 30% (below 30%) \u{2014} plan to replace the batteries"
        );
        assert_eq!(icon, "battery-caution");
        let c = crossing(5, 4.0, Urgency::Critical);
        assert!(crossing_text(&c, AlertBasis::Estimate)
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
            replaced_text(&r).1,
            "Battery 3% \u{2192} 100%; low-battery alerts re-armed"
        );
        assert!(replaced_text(&r).1.contains("low-battery alerts"));
    }

    #[test]
    fn hints_carry_the_kde_identity_and_the_urgency() {
        let n = crossing_notification(&crossing(5, 4.0, Urgency::Critical), AlertBasis::Estimate);
        let h = n.hints();
        let s = |k: &str| match &h[k] {
            Value::Str(s) => s.as_str().to_string(),
            v => panic!("{k}: {v:?}"),
        };
        assert!(!h.contains_key("desktop-entry"));
        assert_eq!(s("x-kde-appname"), "apple-kb-monitor");
        assert_eq!(s("x-kde-eventId"), "BatteryCritical");
        assert_eq!(s("category"), "device");
        assert!(matches!(h["urgency"], Value::U8(2)));
        assert!(
            !h.contains_key("transient"),
            "critical alerts stay in the history"
        );
        assert_eq!(n.event.slot(), "battery");
    }

    #[test]
    fn urgency_timeout_and_transience() {
        let crit =
            crossing_notification(&crossing(5, 4.0, Urgency::Critical), AlertBasis::Estimate);
        assert_eq!(crit.timeout_ms(), 0, "critical = persistent");
        let low = crossing_notification(&crossing(30, 29.0, Urgency::Normal), AlertBasis::Estimate);
        assert_eq!(low.timeout_ms(), 12_000);
        assert!(!low.transient);
        let dis = link_notification(&LinkEvent::Disconnected { mac: "m".into() });
        assert!(dis.transient && dis.urgency == Urgency::Low);
        assert_eq!(dis.timeout_ms(), 6_000);
        assert!(matches!(dis.hints()["transient"], Value::Bool(true)));
        assert_eq!(dis.event.category(), "device.removed");
        let rec = link_notification(&LinkEvent::Reconnected {
            mac: "m".into(),
            pct: None,
        });
        assert_eq!(rec.event, Event::KeyboardReconnected);
        assert_eq!(rec.event.category(), "device.added");
        assert_eq!(
            rec.event.slot(),
            dis.event.slot(),
            "reconnection replaces the disconnection"
        );
        assert_eq!(
            reminder_notification(&rem_low(), Some(10.0)).timeout_ms(),
            15_000
        );
        assert_eq!(
            reminder_notification(&rem_crit(), None).timeout_ms(),
            0,
            "critical: persistent"
        );
    }

    #[test]
    fn events_map_to_the_right_buttons() {
        let list = |n: Notification| n.action_list();
        let rep = notice_notification(Event::RepairNeeded, "s", "b", Urgency::Critical);
        let l = list(rep);
        let keys: Vec<&str> = l
            .iter()
            .step_by(2)
            .map(std::string::String::as_str)
            .collect();
        assert_eq!(keys, ["default", "repair", "open"]);
        let rem = list(reminder_notification(&rem_low(), Some(9.0)));
        assert_eq!(rem[1], "Open");
        assert!(
            rem.contains(&"ignore".to_string())
                && rem.contains(&"Ignore this reminder".to_string())
        );
        let keys: Vec<&str> = rem
            .iter()
            .step_by(2)
            .map(std::string::String::as_str)
            .collect();
        assert_eq!(keys, ["default", "open", "remind", "ignore"]);
        assert!(rem.contains(&"Remind me tomorrow".to_string()));
        assert!(
            generic_notification("s", "b", "dialog-error", Urgency::Critical, false)
                .action_list()
                .is_empty(),
            "{:?}",
            generic_notification("s", "b", "dialog-error", Urgency::Critical, false).action_list()
        );
        assert_eq!(
            list(crossing_notification(
                &crossing(30, 29.0, Urgency::Normal),
                AlertBasis::Estimate
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

    #[test]
    fn the_reapply_offer_says_the_setting_is_common_to_every_apple_keyboard() {
        let fr = reapply_notification("Desk", 2, 1);
        assert_eq!(fr.event, Event::SettingsReapply);
        assert_eq!(fr.action_list(), ["apply", "Apply"]);
        assert!(fr.body.contains("F1\u{2013}F12 first"), "{}", fr.body);
        assert!(fr.body.contains("media keys first"), "{}", fr.body);
        assert!(fr.body.contains("common to every Apple keyboard"));
        assert!(fr.body.contains("authentication"));
        let en = reapply_notification("Desk", 1, 2);
        assert_eq!(en.action_list(), ["apply", "Apply"]);
        assert!(en.body.contains("common to every Apple keyboard"));
        assert!(en
            .body
            .contains("remembered for it: media keys first; in effect: F1\u{2013}F12 first"));
        let mut r = Registry::default();
        r.record("settings", 3, Event::SettingsReapply);
        assert!(
            r.take_action(3, "default").is_none(),
            "a click on the body applies nothing"
        );
        assert_eq!(
            r.take_action(3, "apply"),
            Some((Action::ApplySettings, None))
        );
    }

    #[test]
    fn the_forget_confirmation_offers_one_button_and_no_default() {
        let n = forget_confirm_notification("Alice's keyboard");
        assert_eq!(n.event, Event::ForgetConfirm);
        assert_eq!(n.action_list(), ["forget", "Forget"]);
        assert_eq!(
            n.urgency,
            Urgency::Critical,
            "shown at once, quiet hours or not"
        );
        assert_eq!(n.timeout_ms(), 60_000);
        assert!(
            n.body.contains("removed from this computer") && n.body.contains("within one minute")
        );
        let en = forget_confirm_notification("Kb");
        assert_eq!(en.action_list(), ["forget", "Forget"]);
        assert!(en.body.contains("Nothing is written to the keyboard"));
        let mut r = Registry::default();
        r.record("forget", 5, Event::ForgetConfirm);
        assert!(r.take_action(5, "default").is_none());
        assert!(r.take_action(5, "open").is_none());
        assert_eq!(r.take_action(5, "forget"), Some((Action::Forget, None)));
        for e in Event::ALL {
            assert_eq!(
                e.actions().contains(&Action::Forget),
                e == Event::ForgetConfirm,
                "{e:?}"
            );
        }
    }

    #[test]
    fn unstable_link_text_gives_the_count_and_the_next_step() {
        let fr = unstable_notification("Alice's keyboard", 4);
        assert_eq!(fr.event, Event::LinkUnstable);
        assert_eq!(fr.summary, "Keyboard: unstable link");
        assert!(
            fr.body.contains("4 times within the last hour") && fr.body.contains("akmctl doctor")
        );
        assert_eq!(fr.urgency, Urgency::Normal);
        let en = unstable_notification("Kb <b>", 5);
        assert!(en.body.contains("5 times within the last hour"));
        assert!(
            en.body.contains("&lt;b&gt;"),
            "the name is data: {}",
            en.body
        );
        assert_ne!(
            Event::LinkUnstable.slot(),
            Event::KeyboardDisconnected.slot()
        );
    }

    #[test]
    fn battery_advice_says_what_was_measured_and_what_to_try() {
        let a = akm_core::advice::ShortLife {
            days: [12.0, 18.0],
            since: 1,
        };
        let fr = advice_notification(&a);
        assert_eq!(fr.event, Event::BatteryAdvice);
        assert_eq!(fr.urgency, Urgency::Normal);
        assert!(fr.summary.contains("too often"));
        assert!(
            fr.body.contains("12 d then 18 d") && fr.body.contains("NiMH"),
            "{}",
            fr.body
        );
        let en = advice_notification(&a);
        assert!(en.body.contains("12 d then 18 d") && en.summary.contains("too often"));
        assert_ne!(Event::BatteryAdvice.slot(), Event::BatteryReplaced.slot());
        assert_eq!(fr.action_list(), ["default", "Open", "open", "Open"]);
    }

    #[test]
    fn a_stored_notification_comes_back_identical() {
        for n in [
            crossing_notification(&crossing(30, 29.0, Urgency::Normal), AlertBasis::Estimate),
            reminder_notification(&rem_crit(), Some(4.0)),
            link_notification(&LinkEvent::Disconnected { mac: "m".into() }),
            removed_notification("Alice's <b>own</b> keyboard"),
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
        crossing_notification(&crossing(30, 29.0, Urgency::Normal), AlertBasis::Estimate)
            .to_stored()
    }

    #[test]
    fn event_ids_are_unique() {
        let mut ids: Vec<&str> = Event::ALL.iter().map(|e| e.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Event::ALL.len());
    }

    #[test]
    fn notifyrc_declares_every_event_in_french_and_english() {
        let rc = include_str!("../../../data/apple-kb-monitor.notifyrc");
        assert!(rc.contains("[Global]") && !rc.contains("DesktopEntry="));
        assert!(rc.contains("Name[fr]=") && rc.contains("IconName="));
        let mut sections: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut cur = "";
        for line in rc.lines() {
            if let Some(s) = line
                .strip_prefix("[Event/")
                .and_then(|s| s.strip_suffix(']'))
            {
                cur = s;
                sections.entry(cur).or_default();
            } else if line.starts_with('[') {
                cur = "";
            } else if !cur.is_empty() {
                sections.get_mut(cur).unwrap().push(line);
            }
        }
        for e in Event::ALL {
            let body = sections
                .get(e.id())
                .unwrap_or_else(|| panic!("no [Event/{}]", e.id()));
            for key in ["Name=", "Name[fr]=", "Comment=", "Comment[fr]=", "Action="] {
                assert!(body.iter().any(|l| l.starts_with(key)), "{}: {key}", e.id());
            }
        }
        assert_eq!(
            sections.len(),
            Event::ALL.len(),
            "events in the notifyrc but not in the daemon"
        );
    }

    #[test]
    fn registry_replaces_per_slot_and_checks_the_button() {
        let mut r = Registry::default();
        assert_eq!(r.replace_id("battery"), 0);
        r.record("battery", 7, Event::BatteryLow);
        assert_eq!(r.replace_id("battery"), 7, "next battery alert replaces #7");
        assert_eq!(r.replace_id("link"), 0, "other slots are independent");
        r.record("battery", 9, Event::BatteryCritical);
        assert_eq!(r.replace_id("battery"), 9);
        assert!(r.take_action(7, "open").is_none(), "7 is forgotten");
        assert!(r.take_action(9, "repair").is_none());
        assert!(r.take_action(9, "bogus").is_none());
        r.set_token(9, "tok".into());
        assert_eq!(
            r.take_action(9, "default"),
            Some((Action::Open, Some("tok".into())))
        );
        assert!(
            r.take_action(9, "open").is_none(),
            "one action per notification"
        );
        assert_eq!(r.replace_id("battery"), 0);
        r.record("link", 3, Event::KeyboardDisconnected);
        r.forget(3);
        assert_eq!(r.replace_id("link"), 0);
        for i in 0..200u32 {
            r.record("error", 1000 + i, Event::Error);
        }
        assert!(r.pending.len() <= MAX_PENDING && r.order.len() <= MAX_PENDING);
    }

    #[test]
    fn mute_server_never_blocks_the_caller() {
        use std::time::Instant;
        // jobs that never return must not delay submit(), nor the next ones.
        let n = Notifier::new(Duration::from_millis(100), 2);
        let t = Instant::now();
        for _ in 0..8 {
            n.submit(|| thread::sleep(Duration::from_secs(30)));
        }
        assert!(
            t.elapsed() < Duration::from_millis(500),
            "{:?}",
            t.elapsed()
        );
        // Once the 2 allowed blocked calls are stuck, the rest is dropped, never run, never waited
        // for.
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
        let (s, b) = low_battery_text(12.4);
        assert!(s.contains("Low Battery"));
        assert_eq!(b, "Battery at 12% \u{2014} charge soon");
        assert!(low_battery_text(12.4).1.contains("12%"));
    }

    #[test]
    fn keyboard_driven_alert_texts() {
        use akm_core::registry::BatteryState as B;
        let (sum, b, icon, u) = battery_state_text(B::Low).unwrap();
        assert!(sum.contains("battery low (keyboard alert)"));
        assert!(b.contains("low battery") && icon == "battery-caution" && u == Urgency::Normal);
        let (sum, b, icon, u) = battery_state_text(B::Critical).unwrap();
        assert!(sum.contains("battery critical (keyboard alert)"));
        assert!(
            b.contains("replace the batteries now")
                && icon == "battery-empty"
                && u == Urgency::Critical
        );
        assert!(
            battery_state_text(B::Normal).is_none() && battery_state_text(B::Invalid(9)).is_none()
        );
        assert_eq!(
            battery_state_notification(B::Critical).unwrap().event,
            Event::KeyboardAlert
        );
    }
}
