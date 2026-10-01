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

pub fn plan(kb: Option<&KbFacts>, adapter_powered: Option<bool>, health: Option<&str>, force: bool) -> Plan {
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
    conn.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), "Connect", &())
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
        std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(b).is_file()))
    };
    CANDIDATES.iter().find(|c| in_path(c[0])).map(|c| c.to_vec())
}

fn find_kb(conn: &Connection, mac: &str) -> Option<KbFacts> {
    doctor::bluez_facts(conn)
        .ok()?
        .0
        .into_iter()
        .find(|k| k.mac.eq_ignore_ascii_case(mac))
}

fn re_pair(conn: &Connection, k: Option<&KbFacts>) -> bool {
    if let Some(k) = k {
        let adapter = k.path.rsplit_once('/').map_or("/org/bluez/hci0", |(a, _)| a);
        let obj = zbus::zvariant::ObjectPath::try_from(k.path.as_str()).ok();
        let r = obj.map(|o| {
            conn.call_method(Some("org.bluez"), adapter, Some("org.bluez.Adapter1"), "RemoveDevice", &(o,))
        });
        match r {
            Some(Ok(_)) => say("  pairage supprimé.", "  pairing removed."),
            Some(Err(e)) => {
                println!("  RemoveDevice: {e}");
                return false;
            }
            None => return false,
        }
    }
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
        say("  (attente de l'appairage dans l'assistant…)", "  (waiting for the assistant…)");
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
                say("\u{2713} Clavier ré-appairé, connecté et approuvé.", "\u{2713} Keyboard re-paired, connected and trusted.");
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
                say("Annulé : rien n'a été modifié.", "Cancelled: nothing was changed.");
                crate::cli::EXIT_ERROR
            }
        }
        Plan::Repair(why) => {
            if ask_confirmation(&why, kb.as_ref()) && re_pair(&conn, kb.as_ref()) {
                crate::cli::EXIT_OK
            } else {
                say("Annulé : rien n'a été modifié.", "Cancelled: nothing was changed.");
                crate::cli::EXIT_ERROR
            }
        }
        Plan::PairNew => {
            say("Aucun clavier Apple appairé : lancement de l'assistant.", "No paired Apple keyboard: starting the assistant.");
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
        assert_eq!(plan(Some(&kb(true, true)), Some(true), Some("connected"), false), Plan::Healthy);
        assert_eq!(plan(Some(&kb(true, false)), Some(true), Some("dormant"), false), Plan::WakeAndPage);
        assert_eq!(plan(Some(&kb(true, false)), Some(true), Some("unreachable"), false), Plan::WakeAndPage);
        assert!(matches!(plan(Some(&kb(true, false)), Some(true), Some("auth-failed"), false), Plan::Repair(_)));
        assert!(matches!(plan(Some(&kb(false, false)), Some(true), None, false), Plan::Repair(_)));
        assert!(matches!(plan(Some(&kb(true, true)), Some(true), None, true), Plan::Repair(_)));
        assert_eq!(plan(None, Some(true), None, false), Plan::PairNew);
        assert_eq!(plan(Some(&kb(true, false)), Some(false), None, false), Plan::BluezDown);
        assert_eq!(plan(Some(&kb(true, false)), None, None, false), Plan::BluezDown);
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
