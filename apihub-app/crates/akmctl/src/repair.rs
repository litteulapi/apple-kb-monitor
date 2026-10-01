//! `akmctl repair`: guided recovery of the keyboard's Bluetooth link (#147).
//!
//! 1. Diagnose (read-only). A healthy link is left alone.
//! 2. Non-destructive first: the user presses a key, `Device1.Connect` is
//!    tried a few times, 20 s apart (the keyboard listens a while after a
//!    key press); then the user switches the keyboard OFF and ON (its firmware
//!    can hang after HID request bursts, docs/RECONNEXION-PAIRAGE.md §3.6) and
//!    the pages start again. Most "lost" keyboards come back here: the
//!    pairing was fine.
//! 3. Only with evidence that the pairing is refused (or when step 2 failed
//!    and the user wants to go on), and only after the user TYPES the
//!    confirmation word, the pairing is removed (`Adapter1.RemoveDevice`) and
//!    the desktop pairing assistant is started; the command then waits for
//!    the keyboard to come back paired + connected and marks it trusted.
//!
//! Nothing is ever removed without that typed confirmation; a non-interactive
//! stdin refuses step 3.
//!
//! When the keyboard is still CONNECTED at step 3 (`--force` on a live link),
//! the removal is Apple's clean forget ([`crate::forget`], #217): pre-flight,
//! backup of the host-side facts, explanation + OUBLIER, ONE SET Feature
//! `0x41` `RecantConnection`, 2000 ms, then only `RemoveDevice`; if `0x41` is
//! not accepted nothing is removed and the repair goes back to wake +
//! reconnect. A keyboard that is not connected is unpaired as before (macOS
//! also sends `0x41` only to a connected device).

use std::io::{BufRead, IsTerminal, Write};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::doctor::{self, KbFacts};

/// Typed confirmation required before the pairing is removed.
pub const CONFIRM_FR: &str = "OUBLIER";
pub const CONFIRM_EN: &str = "FORGET";
const ATTEMPTS: u32 = 4;
const SPACING: Duration = Duration::from_secs(20);
const PAIR_WAIT: Duration = Duration::from_secs(240);

/// What the repair should do, from the diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Connected and healthy: nothing to repair.
    Healthy,
    /// Paired, not connected, no evidence against the pairing: wake + page.
    WakeAndPage,
    /// The pairing is refused / missing / removed: re-pair (with consent).
    Repair(String),
    /// No Apple keyboard at all in BlueZ: pairing assistant only.
    PairNew,
    /// BlueZ itself is not usable.
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
        return Plan::Repair("the keyboard is not paired with this computer".into());
    }
    if health == Some("auth-failed") {
        return Plan::Repair("the keyboard refused the stored pairing".into());
    }
    if k.connected && !force {
        return Plan::Healthy;
    }
    if k.connected {
        return Plan::Repair("forced by --force".into());
    }
    Plan::WakeAndPage
}

/// Exact, case-sensitive typed confirmation.
pub fn confirmed(input: &str) -> bool {
    let t = input.trim();
    t == CONFIRM_FR || t == CONFIRM_EN
}

fn fr() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .is_some_and(|v| v.to_ascii_lowercase().starts_with("fr"))
}

fn say(fr_text: &str, en_text: &str) {
    println!("{}", if fr() { fr_text } else { en_text });
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

/// Step 2: key press + pages, then power cycle + pages. True if back.
fn wake_and_page(conn: &Connection, k: &KbFacts) -> bool {
    if press_and_page(conn, k) {
        return true;
    }
    say(
        "\u{2192} Éteignez le clavier (bouton d'alimentation, 3 s, le voyant s'éteint), attendez 5 s, rallumez-le.\n  \
         (Cela ne touche pas au pairage ; c'est souvent ce qui « répare » vraiment.) Entrée quand c'est fait…",
        "\u{2192} Switch the keyboard OFF (power button, 3 s, light off), wait 5 s, switch it ON.\n  \
         (The pairing is untouched; this is often what really fixes it.) Enter when done…",
    );
    let _ = std::io::stdout().flush();
    if std::io::stdin().is_terminal() {
        let mut l = String::new();
        let _ = std::io::stdin().lock().read_line(&mut l);
    }
    press_and_page(conn, k)
}

fn press_and_page(conn: &Connection, k: &KbFacts) -> bool {
    say(
        &format!("\u{2192} Appuyez sur une touche de « {} » maintenant (il se réveille et appelle l'ordinateur).", k.name),
        &format!("\u{2192} Press a key on \u{201c}{}\u{201d} now (it wakes up and calls the computer).", k.name),
    );
    if wait_connected(conn, &k.path, Duration::from_secs(10)) {
        return true;
    }
    for i in 1..=ATTEMPTS {
        say(
            &format!("  appel {i}/{ATTEMPTS} de l'ordinateur vers le clavier…"),
            &format!("  page {i}/{ATTEMPTS} from the computer to the keyboard…"),
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
            say(
                "  (appuyez de nouveau sur une touche)",
                "  (press a key again)",
            );
            if wait_connected(conn, &k.path, SPACING) {
                return true;
            }
        }
    }
    false
}

fn ask_confirmation(why: &str, k: Option<&KbFacts>) -> bool {
    let name = k.map_or("", |k| k.name.as_str());
    say(
        &format!(
            "\nRÉ-APPAIRAGE ({why}).\nCela SUPPRIME le pairage de « {name} » sur cet ordinateur puis lance l'assistant d'appairage.\n\
             Le clavier ne fonctionnera plus jusqu'à la fin de l'appairage : gardez une autre saisie à portée.\n\
             Tapez {CONFIRM_FR} puis Entrée pour continuer, ou seulement Entrée pour annuler : "
        ),
        &format!(
            "\nRE-PAIRING ({why}).\nThis REMOVES the pairing of \u{201c}{name}\u{201d} on this computer, then starts the pairing assistant.\n\
             The keyboard will not work until pairing is done: keep another input device at hand.\n\
             Type {CONFIRM_EN} then Enter to go on, or just Enter to cancel: "
        ),
    );
    let _ = std::io::stdout().flush();
    if !std::io::stdin().is_terminal() {
        say(
            "refusé : confirmation interactive obligatoire (stdin n'est pas un terminal).",
            "refused: interactive confirmation required (stdin is not a terminal).",
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

/// `Adapter1.RemoveDevice` of the keyboard.
fn remove_device(conn: &Connection, k: &KbFacts) -> Result<(), String> {
    let adapter = adapter_of(k);
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

fn adapter_of(k: &KbFacts) -> &str {
    k.path
        .rsplit_once('/')
        .map_or("/org/bluez/hci0", |(a, _)| a)
}

fn re_pair(conn: &Connection, k: Option<&KbFacts>) -> bool {
    if let Some(k) = k {
        match remove_device(conn, k) {
            Ok(()) => say("  pairage supprimé.", "  pairing removed."),
            Err(e) => {
                println!("  RemoveDevice: {e}");
                return false;
            }
        }
    }
    assist_and_wait(conn, k)
}

/// After the removal: pairing assistant, wait for the keyboard paired +
/// connected, mark it trusted.
fn assist_and_wait(conn: &Connection, k: Option<&KbFacts>) -> bool {
    say(
        "\u{2192} Éteignez le clavier (bouton d'alimentation, 3 s), rallumez-le : le voyant clignote (mode appairage).\n\
         \u{2192} Dans l'assistant, choisissez le clavier, tapez sur le CLAVIER le code affiché puis Entrée.",
        "\u{2192} Switch the keyboard off (power button, 3 s) and on again: its light blinks (pairing mode).\n\
         \u{2192} In the assistant, pick the keyboard, type the displayed code ON THE KEYBOARD, then Enter.",
    );
    match pairing_assistant() {
        Some(argv) => {
            if let Err(e) = std::process::Command::new(argv[0]).args(&argv[1..]).spawn() {
                println!("  {}: {e}", argv[0]);
            }
        }
        None => say(
            "  Aucun assistant graphique : `bluetoothctl`, puis `scan on`, `pair <MAC>`, code tapé sur le clavier, `trust <MAC>`, `connect <MAC>`.",
            "  No graphical assistant: `bluetoothctl`, then `scan on`, `pair <MAC>`, type the code on the keyboard, `trust <MAC>`, `connect <MAC>`.",
        ),
    }
    let Some(mac) = k.map(|k| k.mac.clone()) else {
        say(
            "  (attente de l'appairage dans l'assistant…)",
            "  (waiting for the assistant…)",
        );
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
                say(
                    "\u{2713} Clavier ré-appairé, connecté et approuvé.",
                    "\u{2713} Keyboard re-paired, connected and trusted.",
                );
                return true;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    say(
        "\u{2717} Pas de clavier appairé + connecté après 4 min. Relancez `akmctl repair`.",
        "\u{2717} No paired + connected keyboard after 4 min. Run `akmctl repair` again.",
    );
    false
}

// ── clean forget of a connected keyboard (#217) ──────────────────────────

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
            .and_then(|s| s.keyboard.map(|k| k.breaker_open))
            .unwrap_or(false);
        crate::forget::Preflight {
            connected: connected(self.conn, &self.k.path),
            link_healthy: self.link_healthy,
            breaker_open,
            interactive: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        }
    }
    fn save_backup(&mut self) -> Result<std::path::PathBuf, String> {
        let adapter = adapter_of(self.k);
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
        say(
            "\nOUBLI PROPRE, comme macOS : le clavier est connecté, l'ordinateur lui envoie d'abord « RecantConnection »\n\
             (SET Feature 0x41, un octet, fil 53 41), attend 2 s que la liaison tombe, puis SEULEMENT supprime le pairage.\n\
             L'effet exact de 0x41 sur le clavier n'a jamais été mesuré (simple coupure ou oubli de cet ordinateur).\n\
             Ensuite, pour ré-appairer : éteignez le clavier (3 s), rallumez-le (le voyant clignote = mode appairage),\n\
             choisissez-le dans l'assistant et tapez le code affiché SUR LE CLAVIER puis Entrée.",
            "\nCLEAN FORGET, like macOS: the keyboard is connected, the computer first sends it \u{201c}RecantConnection\u{201d}\n\
             (SET Feature 0x41, one byte, wire 53 41), waits 2 s for the link to drop, then ONLY removes the pairing.\n\
             The exact effect of 0x41 on the keyboard was never measured (simple drop or forgetting this computer).\n\
             Then, to re-pair: switch the keyboard off (3 s) and on (blinking light = pairing mode),\n\
             pick it in the assistant and type the displayed code ON THE KEYBOARD, then Enter.",
        );
        ask_confirmation(self.why, Some(self.k))
    }
    fn open_door(&mut self) -> Result<(), String> {
        let d = akm_core::hidraw::WriteDoor::open()?;
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
        // The HID lock is released first: nothing else is written.
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

struct NoDoor;
static NO_DOOR: NoDoor = NoDoor;

impl akm_core::parity::FeatureSink for NoDoor {
    fn set_feature(&self, _op: akm_core::registry::WriteOp, _report: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "no hidraw write door: nothing written",
        ))
    }
}

/// Is the link healthy enough for the forget (doctor: health `connected`, no KO)?
pub fn link_healthy(health: Option<&str>, verdict: doctor::Level) -> bool {
    health == Some("connected") && verdict < doctor::Level::Bad
}

/// Plan::Repair on a CONNECTED keyboard: Apple's clean forget, then the
/// pairing assistant and a final `akmctl doctor`.
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
            say(
                &format!(
                    "  pairage supprimé (liaison tombée : {}), sauvegarde : {}",
                    if link_dropped { "oui" } else { "non" },
                    backup.display()
                ),
                &format!(
                    "  pairing removed (link dropped: {}), backup: {}",
                    if link_dropped { "yes" } else { "no" },
                    backup.display()
                ),
            );
            let ok = assist_and_wait(conn, Some(k));
            let r = doctor::gather(Some(&k.mac));
            println!("akmctl doctor: {:?} - {}", r.verdict.0, r.verdict.1);
            if ok && r.verdict.0 < doctor::Level::Bad {
                crate::cli::EXIT_OK
            } else {
                crate::cli::EXIT_ERROR
            }
        }
        crate::forget::Outcome::RecantFailed(e) | crate::forget::Outcome::NotSent(e) => {
            say(
                &format!("\u{2717} RecantConnection non envoyé ou refusé ({e}) : RIEN n'a été supprimé. Retour à l'étape réveil + reconnexion."),
                &format!("\u{2717} RecantConnection not sent or refused ({e}): NOTHING was removed. Back to wake + reconnect."),
            );
            if wake_and_page(conn, k) {
                say(
                    "\u{2713} Clavier connecté, pairage intact.",
                    "\u{2713} Keyboard connected, pairing intact.",
                );
            }
            crate::cli::EXIT_ERROR
        }
        crate::forget::Outcome::RemoveFailed(e) => {
            println!("  RemoveDevice: {e}");
            crate::cli::EXIT_ERROR
        }
        other => {
            say(
                &format!("Annulé : rien n'a été modifié ({other:?})."),
                &format!("Cancelled: nothing was changed ({other:?})."),
            );
            crate::cli::EXIT_ERROR
        }
    }
}

/// Run the guided repair. Returns the exit code.
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
    let powered = doctor::bluez_facts(&conn).ok().and_then(|x| x.1);
    let kb = report.keyboard.clone();
    let p = plan(kb.as_ref(), powered, report.health.as_deref(), force);
    match p {
        Plan::BluezDown => {
            say(
                "BlueZ ou l'adaptateur est arrêté : rien à ré-appairer. `systemctl status bluetooth`, `bluetoothctl power on`.",
                "BlueZ or the adapter is down: nothing to re-pair. `systemctl status bluetooth`, `bluetoothctl power on`.",
            );
            crate::cli::EXIT_ERROR
        }
        Plan::Healthy => {
            say(
                "La liaison est saine : rien à réparer (--force pour ré-appairer quand même).",
                "The link is healthy: nothing to repair (--force to re-pair anyway).",
            );
            crate::cli::EXIT_OK
        }
        Plan::WakeAndPage => {
            let k = kb.expect("planned with a keyboard");
            if wake_and_page(&conn, &k) {
                say(
                    "\u{2713} Clavier reconnecté : le pairage était intact, aucun ré-appairage nécessaire.",
                    "\u{2713} Keyboard reconnected: the pairing was fine, no re-pairing needed.",
                );
                return crate::cli::EXIT_OK;
            }
            say(
                "\u{2717} Le clavier ne répond pas. Vérifiez qu'il est allumé (voyant au démarrage) et ses piles.",
                "\u{2717} The keyboard does not answer. Check it is switched on (light at power-on) and its batteries.",
            );
            let why = if fr() {
                "le clavier ne répond plus malgré les appels"
            } else {
                "the keyboard does not answer the pages"
            };
            if ask_confirmation(why, Some(&k)) && re_pair(&conn, Some(&k)) {
                crate::cli::EXIT_OK
            } else {
                say(
                    "Annulé : rien n'a été modifié.",
                    "Cancelled: nothing was changed.",
                );
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
                say(
                    "Annulé : rien n'a été modifié.",
                    "Cancelled: nothing was changed.",
                );
                crate::cli::EXIT_ERROR
            }
        }
        Plan::PairNew => {
            say(
                "Aucun clavier Apple appairé : lancement de l'assistant.",
                "No paired Apple keyboard: starting the assistant.",
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

    fn kb(paired: bool, connected: bool) -> KbFacts {
        KbFacts {
            mac: "04:DB:56:CA:42:EE".into(),
            name: "Clavier".into(),
            path: "/org/bluez/hci0/dev_04_DB_56_CA_42_EE".into(),
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
        assert!(confirmed("OUBLIER\n"));
        assert!(confirmed("  FORGET "));
        for no in ["", "\n", "oui", "y", "yes", "oublier", "OUBLIER!", "O"] {
            assert!(!confirmed(no), "{no:?} must not confirm");
        }
    }
}
