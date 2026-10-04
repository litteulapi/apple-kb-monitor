//! `akmctl repair`: guided recovery of the keyboard's Bluetooth link.

use akm_core::tr;
use std::io::{BufRead, IsTerminal, Write};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::doctor::{self, KbFacts};

pub const CONFIRM: &str = "FORGET";
const ATTEMPTS: u32 = 4;
const SPACING: Duration = Duration::from_secs(20);
const PAIR_WAIT: Duration = Duration::from_mins(4);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Connected and healthy: nothing to repair.
    Healthy,
    /// Paired, not connected, no evidence against the pairing: wake + page.
    WakeAndPage,
    /// The pairing is refused / missing / removed: re-pair (with consent).
    Repair(String),
    /// No Apple keyboard at all in `BlueZ`: pairing assistant only.
    PairNew,
    /// `BlueZ` itself is not usable.
    BluezDown,
}

pub fn plan(
    kb: Option<&KbFacts>,
    adapter_powered: Option<bool>,
    health: Option<&str>,
    force: bool,
) -> Plan {
    if adapter_powered != Some(true) {
        return Plan::BluezDown;
    }
    let Some(k) = kb else {
        return Plan::PairNew;
    };
    if !(k.paired || k.bonded) {
        return Plan::Repair(tr!("the keyboard is not paired with this computer"));
    }
    if health == Some("auth-failed") {
        return Plan::Repair(tr!("the keyboard refused the stored pairing"));
    }
    if k.connected && !force {
        return Plan::Healthy;
    }
    if k.connected {
        return Plan::Repair(tr!("forced by --force"));
    }
    Plan::WakeAndPage
}

pub fn confirmed(input: &str) -> bool {
    let t = input.trim();
    t == CONFIRM
}

fn connected(conn: &Connection, path: &str) -> bool {
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &("org.bluez.Device1", "Connected"),
    )
    .ok()
    .and_then(|r| r.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
    .and_then(|v| bool::try_from(&v).ok())
    .unwrap_or(false)
}

fn page(conn: &Connection, path: &str) -> Result<(), String> {
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.bluez.Device1"),
        "Connect",
        &(),
    )
    .map(|_| ())
    .map_err(|e| match e {
        zbus::Error::MethodError(n, m, _) => {
            akm_core::recovery::ConnectError::classify(n.as_str(), m.as_deref().unwrap_or(""))
                .describe()
        }
        o => o.to_string(),
    })
}

fn wait_connected(conn: &Connection, path: &str, max: Duration) -> bool {
    let end = Instant::now() + max;
    while Instant::now() < end {
        if connected(conn, path) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn wake_and_page(conn: &Connection, k: &KbFacts) -> bool {
    if press_and_page(conn, k) {
        return true;
    }
    println!("{}", tr!("\u{2192} Switch the keyboard OFF (power button, 3 s, light off), wait 5 s, switch it ON.\n  \
         (The pairing is untouched; this is often what really fixes it.) Enter when done…"));
    let _ = std::io::stdout().flush();
    if std::io::stdin().is_terminal() {
        let mut l = String::new();
        let _ = std::io::stdin().lock().read_line(&mut l);
    }
    press_and_page(conn, k)
}

fn press_and_page(conn: &Connection, k: &KbFacts) -> bool {
    println!(
        "{}",
        tr!(
            "\u{2192} Press a key on \u{201c}{}\u{201d} now (it wakes up and calls the computer).",
            k.name
        )
    );
    if wait_connected(conn, &k.path, Duration::from_secs(10)) {
        return true;
    }
    for i in 1..=ATTEMPTS {
        println!(
            "{}",
            tr!(
                "  page {i}/{ATTEMPTS} from the computer to the keyboard…",
                i = i,
                ATTEMPTS = ATTEMPTS
            )
        );
        match page(conn, &k.path) {
            Ok(()) => {
                if wait_connected(conn, &k.path, Duration::from_secs(5)) {
                    return true;
                }
            }
            Err(e) => println!("    {e}"),
        }
        if i < ATTEMPTS {
            println!("{}", tr!("  (press a key again)"));
            if wait_connected(conn, &k.path, SPACING) {
                return true;
            }
        }
    }
    false
}

fn ask_confirmation(why: &str, k: Option<&KbFacts>) -> bool {
    let name = k.map_or("", |k| k.name.as_str());
    println!("{}", tr!("\nRE-PAIRING ({why}).\nThis REMOVES the pairing of \u{201c}{name}\u{201d} on this computer, then starts the pairing assistant.\n\
             The keyboard will not work until pairing is done: keep another input device at hand.\n\
             Type {CONFIRM} then Enter to go on, or just Enter to cancel: ", why = why, name = name, CONFIRM = CONFIRM));
    let _ = std::io::stdout().flush();
    if !std::io::stdin().is_terminal() {
        println!(
            "{}",
            tr!("refused: interactive confirmation required (stdin is not a terminal).")
        );
        return false;
    }
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    confirmed(&line)
}

fn pairing_assistant() -> Option<Vec<&'static str>> {
    const CANDIDATES: [&[&str]; 3] = [
        &["bluedevil-wizard"],
        &["blueman-assistant"],
        &["gnome-control-center", "bluetooth"],
    ];
    let in_path = |b: &str| {
        std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(b).is_file()))
    };
    CANDIDATES
        .iter()
        .find(|c| in_path(c[0]))
        .map(|c| c.to_vec())
}

fn find_kb(conn: &Connection, mac: &str) -> Option<KbFacts> {
    doctor::bluez_facts(conn)
        .ok()?
        .0
        .into_iter()
        .find(|k| k.mac.eq_ignore_ascii_case(mac))
}

fn remove_device(conn: &Connection, k: &KbFacts) -> Result<(), String> {
    let adapter = doctor::adapter_of(k);
    let obj = zbus::zvariant::ObjectPath::try_from(k.path.as_str()).map_err(|e| e.to_string())?;
    conn.call_method(
        Some("org.bluez"),
        adapter,
        Some("org.bluez.Adapter1"),
        "RemoveDevice",
        &(obj,),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn re_pair(conn: &Connection, k: Option<&KbFacts>) -> bool {
    if let Some(k) = k {
        match remove_device(conn, k) {
            Ok(()) => println!("{}", tr!("  pairing removed.")),
            Err(e) => {
                println!(
                    "{}",
                    tr!("  pairing not removed (RemoveDevice: {e})", e = e)
                );
                return false;
            }
        }
    }
    assist_and_wait(conn, k)
}

fn assist_and_wait(conn: &Connection, k: Option<&KbFacts>) -> bool {
    println!("{}", tr!("\u{2192} Switch the keyboard off (power button, 3 s) and on again: its light blinks (pairing mode).\n\
         \u{2192} In the assistant, pick the keyboard, type the displayed code ON THE KEYBOARD, then Enter."));
    match pairing_assistant() {
        Some(argv) => {
            if let Err(e) = std::process::Command::new(argv[0]).args(&argv[1..]).spawn() {
                println!("  {}: {e}", argv[0]);
            }
        }
        None => println!("{}", tr!("  No graphical assistant: `bluetoothctl`, then `scan on`, `pair <MAC>`, type the code on the keyboard, `trust <MAC>`, `connect <MAC>`.")),
    }
    let Some(mac) = k.map(|k| k.mac.clone()) else {
        println!("{}", tr!("  (waiting for the assistant…)"));
        return true;
    };
    let end = Instant::now() + PAIR_WAIT;
    while Instant::now() < end {
        if let Some(n) = find_kb(conn, &mac) {
            if (n.paired || n.bonded) && n.connected {
                if !n.trusted {
                    let _ = conn.call_method(
                        Some("org.bluez"),
                        n.path.as_str(),
                        Some("org.freedesktop.DBus.Properties"),
                        "Set",
                        &("org.bluez.Device1", "Trusted", Value::from(true)),
                    );
                }
                println!(
                    "{}",
                    tr!("\u{2713} Keyboard re-paired, connected and trusted.")
                );
                return true;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    println!(
        "{}",
        tr!("\u{2717} No paired + connected keyboard after 4 min. Run `akmctl repair` again.")
    );
    false
}

fn bluez_prop(conn: &Connection, path: &str, iface: &str, name: &str) -> Option<String> {
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(iface, name),
    )
    .ok()
    .and_then(|r| r.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
    .and_then(|v| <&str>::try_from(&v).ok().map(str::to_string))
}

struct RealForget<'a> {
    conn: &'a Connection,
    k: &'a KbFacts,
    why: &'a str,
    link_healthy: bool,
    door: Option<akm_core::hidraw::WriteDoor>,
}

impl crate::forget::ForgetEnv for RealForget<'_> {
    fn preflight(&mut self) -> crate::forget::Preflight {
        let breaker_open = crate::bus::connect()
            .ok()
            .and_then(|c| crate::bus::get_state(&c).ok())
            .and_then(|s| daemon_breaker_of(&s, &self.k.mac))
            .unwrap_or_else(|| crate::forget::published_breaker_blocks(&self.k.mac));
        crate::forget::Preflight {
            connected: connected(self.conn, &self.k.path),
            link_healthy: self.link_healthy,
            breaker_open,
            interactive: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        }
    }
    fn save_backup(&mut self) -> Result<std::path::PathBuf, String> {
        let adapter = doctor::adapter_of(self.k);
        let b = crate::forget::ForgetBackup {
            schema: crate::forget::ForgetBackup::SCHEMA,
            created_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            mac: self.k.mac.clone(),
            name: self.k.name.clone(),
            alias: bluez_prop(self.conn, &self.k.path, "org.bluez.Device1", "Alias"),
            paired: self.k.paired,
            bonded: self.k.bonded,
            trusted: self.k.trusted,
            adapter_path: adapter.to_string(),
            adapter_address: bluez_prop(self.conn, adapter, "org.bluez.Adapter1", "Address"),
            device_path: self.k.path.clone(),
        };
        crate::forget::write_backup(&akm_core::devname::state_dir(), &b).map_err(|e| e.to_string())
    }
    fn explain_and_confirm(&mut self) -> bool {
        println!("{}", tr!("\nCLEAN FORGET, like macOS: the keyboard is connected, the computer first sends it \u{201c}RecantConnection\u{201d}\n\
             (SET Feature 0x41, one byte, wire 53 41), waits 2 s for the link to drop, then ONLY removes the pairing.\n\
             The exact effect of 0x41 on the keyboard was never measured (simple drop or forgetting this computer).\n\
             Then, to re-pair: switch the keyboard off (3 s) and on (blinking light = pairing mode),\n\
             pick it in the assistant and type the displayed code ON THE KEYBOARD, then Enter."));
        ask_confirmation(self.why, Some(self.k))
    }
    fn open_door(&mut self) -> Result<(), String> {
        let d = akm_core::hidraw::WriteDoor::open_for(&self.k.mac)?;
        eprintln!("[forget] write door: {}", d.path());
        self.door = Some(d);
        Ok(())
    }
    fn sink(&self) -> &dyn akm_core::parity::FeatureSink {
        match &self.door {
            Some(d) => d,
            None => &NO_DOOR,
        }
    }
    fn expect_disconnect(&mut self) {
        let r = crate::bus::connect()
            .map_err(|e| e.to_string())
            .and_then(|c| crate::bus::expect_disconnect(&c).map_err(|e| e.to_string()));
        if let Err(e) = r {
            eprintln!("[forget] daemon not told (a disconnection notification may appear): {e}");
        }
    }
    fn sleep(&mut self, d: Duration) {
        self.door = None;
        std::thread::sleep(d);
    }
    fn connected(&mut self) -> bool {
        connected(self.conn, &self.k.path)
    }
    fn remove_device(&mut self) -> Result<(), String> {
        remove_device(self.conn, self.k)
    }
    fn log(&mut self, line: &str) {
        eprintln!("{line}");
    }
}

/// The daemon's breaker, only when the daemon follows the keyboard being forgotten.
fn daemon_breaker_of(s: &akm_core::Snapshot, mac: &str) -> Option<bool> {
    crate::devnamecmd::snapshot_mac(s)
        .filter(|d| d.eq_ignore_ascii_case(mac))
        .and_then(|_| s.keyboard.as_ref().map(|k| k.breaker_open))
}

struct NoDoor;
static NO_DOOR: NoDoor = NoDoor;

impl akm_core::parity::FeatureSink for NoDoor {
    fn set_feature(&self, _op: akm_core::registry::WriteOp, _report: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "no hidraw write door: nothing written",
        ))
    }
}

pub fn link_healthy(health: Option<&str>, verdict: doctor::Level) -> bool {
    health == Some("connected") && verdict < doctor::Level::Bad
}

fn forget_connected(conn: &Connection, k: &KbFacts, why: &str, report: &doctor::Report) -> u8 {
    let mut env = RealForget {
        conn,
        k,
        why,
        link_healthy: link_healthy(report.health.as_deref(), report.verdict.0),
        door: None,
    };
    let mut session = akm_core::registry::WriteSession::new();
    match crate::forget::run(&mut session, &mut env) {
        crate::forget::Outcome::Removed {
            link_dropped,
            backup,
        } => {
            println!(
                "{}",
                tr!(
                    "  pairing removed (link dropped: {}), backup: {}",
                    if link_dropped { tr!("yes") } else { tr!("no") },
                    backup.display()
                )
            );
            let ok = assist_and_wait(conn, Some(k));
            let r = doctor::gather(Some(&k.mac));
            println!("akmctl doctor: {} {}", r.verdict.0.tag(), r.verdict.1);
            if ok && r.verdict.0 < doctor::Level::Bad {
                crate::cli::EXIT_OK
            } else {
                crate::cli::EXIT_ERROR
            }
        }
        crate::forget::Outcome::RecantFailed(e) | crate::forget::Outcome::NotSent(e) => {
            println!("{}", tr!("\u{2717} RecantConnection not sent or refused ({e}): NOTHING was removed. Back to wake + reconnect.", e = e));
            if wake_and_page(conn, k) {
                println!("{}", tr!("\u{2713} Keyboard connected, pairing intact."));
            }
            crate::cli::EXIT_ERROR
        }
        crate::forget::Outcome::RemoveFailed(e) => {
            println!(
                "{}",
                tr!("  pairing not removed (RemoveDevice: {e})", e = e)
            );
            crate::cli::EXIT_ERROR
        }
        crate::forget::Outcome::Preflight(fails)
            if crate::forget::after_preflight(&fails) == crate::forget::Next::WakeAndPage =>
        {
            println!("{}", tr!("\u{2717} Circuit breaker open: the keyboard did not answer 3 requests in a row, it is mute (Apple's R3). RecantConnection (0x41) NOT sent, nothing removed. Back to wake + reconnect."));
            if wake_and_page(conn, k) {
                println!("{}", tr!("\u{2713} Keyboard reconnected, pairing intact: the breaker closes on the new connection; run `akmctl repair` again if the forget is still needed."));
                return crate::cli::EXIT_OK;
            }
            println!("{}", tr!("\u{2717} The keyboard does not answer. Switch it off and on (light at power-on), check its batteries, then run `akmctl repair` again."));
            crate::cli::EXIT_ERROR
        }
        other => {
            println!("{}", cancelled_text(&other));
            crate::cli::EXIT_ERROR
        }
    }
}

/// The forget outcomes that change nothing, as one translated sentence.
fn cancelled_text(o: &crate::forget::Outcome) -> String {
    use crate::forget::Outcome;
    match o {
        Outcome::BackupFailed(e) => tr!(
            "Cancelled: the backup could not be written ({e}); nothing was changed.",
            e = e
        ),
        Outcome::Preflight(fails) => tr!(
            "Cancelled: nothing was changed ({other}).",
            other = fails
                .iter()
                .map(|f| akm_core::i18n::gettext(f.describe()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => tr!("Cancelled: nothing was changed."),
    }
}

pub fn run(mac: Option<&str>, force: bool) -> u8 {
    let report = doctor::gather(mac);
    print!("{}", doctor::to_text(&report));
    println!();
    let conn = match Connection::system() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("akmctl: system bus: {e}");
            return crate::cli::EXIT_ERROR;
        }
    };
    let kb = report.keyboard.clone();
    let powered = doctor::bluez_facts(&conn)
        .ok()
        .and_then(|(_, a)| doctor::adapter_powered(&a, kb.as_ref()));
    let p = plan(kb.as_ref(), powered, report.health.as_deref(), force);
    match p {
        Plan::BluezDown => {
            println!("{}", tr!("BlueZ or the adapter is down: nothing to re-pair. `systemctl status bluetooth`, `bluetoothctl power on`."));
            crate::cli::EXIT_ERROR
        }
        Plan::Healthy => {
            println!(
                "{}",
                tr!("The link is healthy: nothing to repair (--force to re-pair anyway).")
            );
            crate::cli::EXIT_OK
        }
        Plan::WakeAndPage => {
            let k = kb.expect("planned with a keyboard");
            if wake_and_page(&conn, &k) {
                println!("{}", tr!("\u{2713} Keyboard reconnected: the pairing was fine, no re-pairing needed."));
                return crate::cli::EXIT_OK;
            }
            println!("{}", tr!("\u{2717} The keyboard does not answer. Check it is switched on (light at power-on) and its batteries."));
            let why = tr!("the keyboard does not answer the pages");
            if ask_confirmation(&why, Some(&k)) && re_pair(&conn, Some(&k)) {
                crate::cli::EXIT_OK
            } else {
                println!("{}", tr!("Cancelled: nothing was changed."));
                crate::cli::EXIT_ERROR
            }
        }
        Plan::Repair(why) => {
            if let Some(k) = kb.as_ref().filter(|k| connected(&conn, &k.path)) {
                return forget_connected(&conn, k, &why, &report);
            }
            if ask_confirmation(&why, kb.as_ref()) && re_pair(&conn, kb.as_ref()) {
                crate::cli::EXIT_OK
            } else {
                println!("{}", tr!("Cancelled: nothing was changed."));
                crate::cli::EXIT_ERROR
            }
        }
        Plan::PairNew => {
            println!(
                "{}",
                tr!("No paired Apple keyboard: starting the assistant.")
            );
            if re_pair(&conn, None) {
                crate::cli::EXIT_OK
            } else {
                crate::cli::EXIT_ERROR
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_forget_shows_no_debug_output() {
        use crate::forget::{Outcome, PreflightFail};
        for o in [
            Outcome::Cancelled,
            Outcome::BackupFailed("No space left on device (os error 28)".into()),
            Outcome::Preflight(vec![
                PreflightFail::NotConnected,
                PreflightFail::LinkNotHealthy,
                PreflightFail::NotInteractive,
            ]),
        ] {
            let text = cancelled_text(&o);
            assert!(text.starts_with("Cancelled: "), "{text}");
            assert!(!text.contains(&format!("({o:?})")), "{text}");
            assert!(!text.contains("Not") && !text.contains('['), "{text}");
        }
        assert_eq!(
            cancelled_text(&Outcome::Preflight(vec![PreflightFail::NotInteractive])),
            "Cancelled: nothing was changed (not an interactive terminal)."
        );
    }

    #[test]
    fn preflight_breaker_is_the_target_keyboard_s_only() {
        let mut k = akm_core::report::KbReport::default();
        k.device.mac = Some("AA:BB:CC:DD:EE:0A".into());
        k.breaker_open = true;
        let s = akm_core::Snapshot {
            keyboard: Some(k),
            ..Default::default()
        };
        assert_eq!(daemon_breaker_of(&s, "aa:bb:cc:dd:ee:0a"), Some(true));
        assert_eq!(daemon_breaker_of(&s, "AA:BB:CC:DD:EE:F1"), None);
        assert_eq!(
            daemon_breaker_of(&akm_core::Snapshot::default(), "AA:BB:CC:DD:EE:F1"),
            None
        );
    }

    fn kb(paired: bool, connected: bool) -> KbFacts {
        KbFacts {
            mac: "AA:BB:CC:DD:EE:F1".into(),
            name: "Keyboard".into(),
            path: "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1".into(),
            paired,
            bonded: paired,
            trusted: true,
            connected,
            ..Default::default()
        }
    }

    #[test]
    fn plans() {
        assert_eq!(
            plan(Some(&kb(true, true)), Some(true), Some("connected"), false),
            Plan::Healthy
        );
        assert_eq!(
            plan(Some(&kb(true, false)), Some(true), Some("dormant"), false),
            Plan::WakeAndPage
        );
        assert_eq!(
            plan(
                Some(&kb(true, false)),
                Some(true),
                Some("unreachable"),
                false
            ),
            Plan::WakeAndPage
        );
        assert!(matches!(
            plan(
                Some(&kb(true, false)),
                Some(true),
                Some("auth-failed"),
                false
            ),
            Plan::Repair(_)
        ));
        assert!(matches!(
            plan(Some(&kb(false, false)), Some(true), None, false),
            Plan::Repair(_)
        ));
        assert!(matches!(
            plan(Some(&kb(true, true)), Some(true), None, true),
            Plan::Repair(_)
        ));
        assert_eq!(plan(None, Some(true), None, false), Plan::PairNew);
        assert_eq!(
            plan(Some(&kb(true, false)), Some(false), None, false),
            Plan::BluezDown
        );
        assert_eq!(
            plan(Some(&kb(true, false)), None, None, false),
            Plan::BluezDown
        );
    }

    #[test]
    fn link_health_for_the_forget() {
        assert!(link_healthy(Some("connected"), doctor::Level::Warn));
        assert!(!link_healthy(Some("connected"), doctor::Level::Bad));
        for h in [
            None,
            Some("dormant"),
            Some("unreachable"),
            Some("auth-failed"),
        ] {
            assert!(!link_healthy(h, doctor::Level::Ok), "{h:?}");
        }
    }

    #[test]
    fn confirmation_is_strict() {
        assert!(!confirmed("OUBLIER\n"));
        assert!(confirmed("  FORGET "));
        for no in ["", "\n", "oui", "y", "yes", "oublier", "OUBLIER!", "O"] {
            assert!(!confirmed(no), "{no:?} must not confirm");
        }
    }
}
