//! `akmctl rename --device-name ...`: the name stored IN the keyboard (#248).
//!
//! Two names exist (docs/RENOMMER-CLAVIER.md):
//! * `akmctl rename <nom>`: alias of THIS computer (BlueZ `Alias`), the
//!   default, risk-free, unchanged;
//! * `akmctl rename --device-name ...`: the keyboard's own name, stored in its
//!   firmware (`0x51`-`0x54` read, `0x55` written by Lion), seen by every host.
//!
//! Four modes:
//! * `--show`: the daemon's cache (no hardware request);
//! * `--dry-run` (the default): validates the name, shows the pre-flight
//!   without the MTU (it needs `pkexec`), saves the current name (backup
//!   0600) and prints every byte that would be sent with its level of proof;
//! * `--check`: the WHOLE pre-flight, control-channel MTU included (one
//!   `pkexec akm-hid-control inspect`, read-only), the backup, the plan, then
//!   stops where the consent would be asked. Nothing is written, nothing is
//!   asked;
//! * `--write-device-name`: the guarded sequence of [`akm_core::devname::run`],
//!   which writes only once THREE locks are lifted.
//!
//! The three locks (none removed, lock 1 gained an interactive form):
//! 1. **consent**: `[apple] allow_device_name_write = true` in `config.toml`
//!    (the only way for automation, no terminal) **or**, in an interactive
//!    terminal (stdin AND stdout are a TTY) when that key is absent/false,
//!    the word `ECRIRE` typed exactly (case-sensitive) after the plan, the
//!    backup and the unmeasured risks were shown. That consent is for THIS
//!    run only: nothing is written into `config.toml`. Without a terminal and
//!    without the key: refused before any pre-flight, as before;
//! 2. the outgoing MTU of the L2CAP control channel, read here through
//!    `pkexec akm-hid-control inspect --mac <MAC>` (read-only, once per
//!    command), ≥ 66;
//! 3. the name typed again in the terminal.
//!
//! No D-Bus method, no window nor tray button can reach this: only this
//! interactive command. The Settings module only opens a terminal on it.
//!
//! Messages are in French when `LC_ALL`/`LC_MESSAGES`/`LANG` starts with `fr`,
//! in English otherwise. Exit codes: see [`EXIT_LOCK1`] and the following.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use akm_core::devname::{
    self, Backup, Frame, Outcome, Preflight, PreflightFail, Reconnect, RenameEnv, Request,
};
use akm_core::parity::FeatureSink;
use akm_core::registry::WriteSession;
use akm_core::Snapshot;

use crate::bus;
use crate::cli::{EXIT_ABSENT, EXIT_ERROR, EXIT_OK};

/// Refused at lock 1: the configuration key is not `true` and no interactive
/// consent is possible (no terminal). Nothing was touched.
pub const EXIT_LOCK1: u8 = 10;
/// Refused by the pre-flight (lock 2, the control-channel MTU, included).
/// Nothing was written.
pub const EXIT_PREFLIGHT: u8 = 11;
/// Cancelled at the keyboard: `ECRIRE` not typed, or the name typed again
/// differs. Nothing was written (the backup stays).
pub const EXIT_CANCELLED: u8 = 12;
/// Written, but the keyboard did not reconnect within the delay.
pub const EXIT_NO_RECONNECT: u8 = 13;
/// Written, read back DIFFERENT: the exact rollback command was printed.
pub const EXIT_MISMATCH: u8 = 14;

/// The word that lifts lock 1 for one run (case-sensitive, no accent).
pub const CONSENT_WORD: &str = "ECRIRE";

/// What the user asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show,
    Rename { name: String, mode: Mode },
    Restore { file: PathBuf, mode: Mode },
}

/// How far the command goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Validate, show the bytes, pre-flight without MTU, backup. No pkexec.
    DryRun,
    /// The whole pre-flight (one pkexec for the MTU), backup, the plan; stops
    /// where the consent would be asked. Nothing written, nothing asked.
    Check,
    /// The guarded write.
    Write,
}

/// Language of the messages (French if the locale starts with `fr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub fn detect() -> Self {
        Self::from_env(|k| std::env::var(k).ok())
    }

    /// `LC_ALL`, then `LC_MESSAGES`, then `LANG`: the first non-empty value.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let fr = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| get(k))
            .find(|v| !v.is_empty())
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("fr"));
        if fr {
            Self::Fr
        } else {
            Self::En
        }
    }

    fn t<'a>(self, fr: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Fr => fr,
            Self::En => en,
        }
    }
}

const TWO_NAMES: &str = "Two names exist:\n\
  * alias on THIS computer (BlueZ Alias): `akmctl rename <name>`, the default, no risk, nothing written into the keyboard;\n\
  * name stored IN the keyboard (firmware, reports 0x51-0x54 / 0x55), seen by every host: `akmctl rename --device-name`.\n";

const TWO_NAMES_FR: &str = "Deux noms existent :\n\
  * l'alias sur CE poste (BlueZ Alias) : `akmctl rename <nom>`, la voie par défaut, sans risque, rien n'est écrit dans le clavier ;\n\
  * le nom stocké DANS le clavier (micrologiciel, rapports 0x51-0x54 / 0x55), vu par tous les hôtes : `akmctl rename --device-name`.\n";

// ── the outside world and the terminal (simulated in the tests) ───────────

/// The terminal: what the user sees and types.
pub trait Io {
    /// stdin AND stdout are a terminal.
    fn interactive(&self) -> bool;
    /// One line typed by the user (`None` = end of input or error).
    fn read_line(&mut self) -> Option<String>;
    /// Text for the user (stdout), printed as is.
    fn out(&mut self, s: &str);
    /// One line of the decision journal (stderr).
    fn journal(&mut self, s: &str);
}

/// Everything the flow needs from outside: daemon, doctor, the read-only MTU
/// probe (`pkexec`), the hidraw door, the state directory, the clock.
pub trait World {
    fn snapshot(&mut self) -> Result<Snapshot, (u8, String)>;
    fn doctor_green(&mut self, mac: Option<&str>) -> bool;
    /// `pkexec akm-hid-control inspect --mac MAC`: the outgoing MTU of the
    /// control channel and the helper's text (read-only, nothing sent).
    fn probe_control_mtu(&mut self, mac: &str) -> Result<(u16, String), String>;
    /// Open the hidraw node under the HID lock (writes nothing by itself).
    fn open_door(&mut self) -> Result<Box<dyn FeatureSink>, String>;
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf>;
    fn now_unix(&mut self) -> u64;
    fn sleep(&mut self, d: Duration);
}

struct Stdio;

impl Io for Stdio {
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
    fn out(&mut self, s: &str) {
        print!("{s}");
        let _ = std::io::stdout().flush();
    }
    fn journal(&mut self, s: &str) {
        eprintln!("{s}");
    }
}

struct RealWorld;

impl World for RealWorld {
    fn snapshot(&mut self) -> Result<Snapshot, (u8, String)> {
        let conn = bus::connect().map_err(|e| (EXIT_ABSENT, e.to_string()))?;
        bus::get_state(&conn).map_err(|e| match e {
            bus::BusError::Absent(m) => (EXIT_ABSENT, m),
            bus::BusError::Failed(m) => (EXIT_ERROR, m),
        })
    }
    fn doctor_green(&mut self, mac: Option<&str>) -> bool {
        let r = crate::doctor::gather(mac);
        r.verdict.0 <= crate::doctor::Level::Info
            && r.keyboard.as_ref().is_some_and(|k| k.connected)
            && r.health.as_deref() == Some("connected")
    }
    fn probe_control_mtu(&mut self, mac: &str) -> Result<(u16, String), String> {
        crate::hid_control::inspect_control_mtu(mac)
    }
    fn open_door(&mut self) -> Result<Box<dyn FeatureSink>, String> {
        akm_core::hidraw::WriteDoor::open().map(|d| Box::new(d) as Box<dyn FeatureSink>)
    }
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
        devname::write_backup(&devname::state_dir(), b)
    }
    fn now_unix(&mut self) -> u64 {
        now_unix()
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The cached 32 bytes of `0x51`-`0x54` in a snapshot.
pub fn cached_raw(s: &Snapshot) -> Option<Vec<u8>> {
    s.keyboard
        .as_ref()
        .and_then(|k| k.device.name_on_keyboard_hex.as_deref())
        .and_then(devname::unhex)
        .filter(|r| r.len() == devname::MAX_NAME_LEN)
}

/// The keyboard's MAC: `--mac`, else the daemon's.
fn snapshot_mac(s: &Snapshot) -> Option<String> {
    s.mac()
        .map(str::to_string)
        .or_else(|| s.keyboard.as_ref().and_then(|k| k.device.mac.clone()))
}

/// Pre-flight facts from a snapshot (+ doctor verdict, terminal, MTU probe).
pub fn preflight_from(
    s: &Snapshot,
    doctor_green: bool,
    interactive: bool,
    control_mtu: Option<u16>,
) -> Preflight {
    let k = s.keyboard.as_ref();
    Preflight {
        connected: s.connected,
        battery_pct: s.battery_pct(),
        // Input 0x30 is only listened to by the daemon, not published: the
        // percentage decides (state `normal` stays unknown here).
        battery_state_normal: None,
        breaker_open: k.is_some_and(|k| k.breaker_open),
        recent_read_failure: k.is_some_and(|k| k.incomplete) || s.kb_error.is_some(),
        doctor_green,
        interactive,
        control_mtu,
    }
}

/// Lock 1, as read from `config.toml` (default false).
fn config_allows_write() -> bool {
    akm_core::config::load(&akm_core::config::default_path())
        .0
        .allow_device_name_write
}

pub fn run(action: Action, mac: Option<String>) -> u8 {
    let lang = Lang::detect();
    let mut world = RealWorld;
    let mut io = Stdio;
    dispatch(
        action,
        mac,
        config_allows_write(),
        lang,
        &mut world,
        &mut io,
    )
}

/// The whole command on a given world and terminal (what the tests drive).
pub fn dispatch(
    action: Action,
    mac: Option<String>,
    config_allowed: bool,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    match action {
        Action::Show => show(lang, world, io),
        Action::Rename { name, mode } => rename(&name, mode, mac, config_allowed, lang, world, io),
        Action::Restore { file, mode } => {
            restore(&file, mode, mac, config_allowed, lang, world, io)
        }
    }
}

fn show(lang: Lang, world: &mut dyn World, io: &mut dyn Io) -> u8 {
    let s = match world.snapshot() {
        Ok(s) => s,
        Err((code, m)) => {
            io.journal(&format!("akmctl: {m}"));
            return code;
        }
    };
    io.out(lang.t(TWO_NAMES_FR, TWO_NAMES));
    io.out(&format!(
        "{} {}\n",
        lang.t("Alias (ce poste) :", "Alias (this computer):"),
        s.alias().unwrap_or(lang.t("(aucun)", "(none)"))
    ));
    match s
        .keyboard
        .as_ref()
        .and_then(|k| k.device.name_on_keyboard.clone())
    {
        Some(n) => {
            io.out(&format!(
                "{} {n}\n",
                lang.t(
                    "Stocké dans le clavier (0x51-0x54, cache du démon) :",
                    "Stored in the keyboard (0x51-0x54, daemon cache):"
                )
            ));
            if let Some(h) = cached_raw(&s) {
                io.out(&format!(
                    "  {} {}\n",
                    lang.t("octets :", "bytes:"),
                    devname::hex(&h)
                ));
            }
            EXIT_OK
        }
        None => {
            io.out(lang.t(
                "Stocké dans le clavier : pas encore lu dans cette connexion (le démon lit 0x51-0x54 une fois par connexion ; aucune lecture matérielle n'est faite ici)\n",
                "Stored in the keyboard: not read yet in this connection (the daemon reads 0x51-0x54 once per connection; no new hardware read is made here)\n",
            ));
            EXIT_ERROR
        }
    }
}

fn proof_state(lang: Lang) -> String {
    let mut s = format!(
        "{} {}.\n{}\n",
        lang.t("Preuve de la séquence :", "Proof of the sequence:"),
        devname::SEQUENCE_PROOF.describe(),
        lang.t(
            "Tranché par désassemblage (docs/RE-NOM-PROPRE-E1.md) :",
            "Settled by disassembly (docs/RE-NOM-PROPRE-E1.md):"
        )
    );
    for u in devname::RESOLVED_BY_DISASSEMBLY {
        s.push_str(&format!("  - {u}\n"));
    }
    s.push_str(&risks(lang));
    s.push_str(lang.t("Expériences :\n", "Experiments:\n"));
    for e in devname::VALIDATION_EXPERIMENTS {
        s.push_str(&format!("  - {e}\n"));
    }
    s
}

/// The two unmeasured risks, in the user's language, then akm-core's list.
fn risks(lang: Lang) -> String {
    let mut s = String::from(lang.t(
        "RISQUES NON MESURÉS (aucun désassemblage ne peut les donner ; docs/RENOMMER-CLAVIER.md §5.3, §6) :\n\
         \x20 - U5 : la réponse du micrologiciel à un SET 0x55 est inconnue (Apple attend un HANDSHAKE réussi sous 1 s ; une erreur serait visible dans [hid-write] failed) ;\n\
         \x20 - U3 : la persistance du nom au changement de piles n'est pas garantie (le 0x55 du Magic Keyboard est déclaré volatile) ; la sauvegarde permet de réécrire.\n\
         \x20 Avoir un second clavier fonctionnel avant d'écrire.\n",
        "RISKS NOT MEASURED (no disassembly can give them; docs/RENOMMER-CLAVIER.md §5.3, §6):\n\
         \x20 - U5: the firmware's answer to a SET 0x55 is unknown (Apple waits for a successful HANDSHAKE within 1 s; an error would show as [hid-write] failed);\n\
         \x20 - U3: persistence of the name across a battery change is not guaranteed (the Magic Keyboard declares its 0x55 volatile); the backup allows rewriting it.\n\
         \x20 Have a second working keyboard before writing.\n",
    ));
    for u in devname::UNKNOWNS {
        s.push_str(&format!("  - {u}\n"));
    }
    s
}

fn locks_state(lang: Lang, allow: bool, interactive: bool) -> String {
    let lock1 = match (allow, interactive) {
        (true, _) => lang.t(
            "levé par config.toml ([apple] allow_device_name_write = true)",
            "lifted by config.toml ([apple] allow_device_name_write = true)",
        ),
        (false, true) => lang.t(
            "config.toml fermé (défaut) : dans ce terminal, le mot ECRIRE tapé après le plan vaut consentement pour CETTE exécution seulement (rien n'est écrit dans config.toml)",
            "config.toml closed (default): in this terminal, typing ECRIRE after the plan is the consent for THIS run only (nothing is written into config.toml)",
        ),
        (false, false) => lang.t(
            "config.toml fermé (défaut) et pas de terminal : refus. Automatisation : [apple] allow_device_name_write = true dans ~/.config/apple-kb-monitor/config.toml ; sinon lancer la commande dans un terminal",
            "config.toml closed (default) and no terminal: refused. Automation: [apple] allow_device_name_write = true in ~/.config/apple-kb-monitor/config.toml; otherwise run the command in a terminal",
        ),
    };
    format!(
        "{}\n  1. {} {lock1}\n  2. {} {} {}\n  3. {}\n",
        lang.t(
            "Trois verrous, tous nécessaires pour une écriture réelle :",
            "Three locks, all required for a real write:"
        ),
        lang.t("consentement :", "consent:"),
        lang.t(
            "MTU sortante du canal de contrôle L2CAP >=",
            "outgoing MTU of the L2CAP control channel >="
        ),
        devname::MIN_CONTROL_MTU,
        lang.t(
            ": lue sur la socket vivante (`pkexec akm-hid-control inspect --mac <MAC>`, authentification administrateur, getsockopt en lecture seule, rien n'est envoyé) ; inconnue ou plus petite = refus",
            ": read on the live socket (`pkexec akm-hid-control inspect --mac <MAC>`, administrator authentication, getsockopt read-only, nothing sent); unknown or smaller = refused"
        ),
        lang.t(
            "le nom retapé exactement, dans un terminal interactif",
            "the name typed again, exactly, in an interactive terminal"
        ),
    )
}

fn fail_text(f: &PreflightFail, lang: Lang) -> String {
    if lang == Lang::En {
        return f.describe();
    }
    match f {
        PreflightFail::NotConnected => "clavier non connecté".into(),
        PreflightFail::Battery => "batterie sous 20 % et état différent de « normal » (ou inconnu)".into(),
        PreflightFail::BreakerOpen => "disjoncteur ouvert : le clavier a cessé de répondre".into(),
        PreflightFail::RecentReadFailure => "la dernière lecture matérielle a échoué ou est incomplète".into(),
        PreflightFail::DoctorNotGreen => "`akmctl doctor` n'est pas vert (liaison, appairage, configuration)".into(),
        PreflightFail::NotInteractive => "pas de terminal interactif (stdin et stdout doivent être un terminal)".into(),
        PreflightFail::ControlMtuUnknown => format!(
            "MTU sortante du canal de contrôle L2CAP inconnue : elle doit être lue (akm-hid-control inspect, lecture seule) et valoir >= {} avant d'envoyer une trame de 66 octets",
            devname::MIN_CONTROL_MTU
        ),
        PreflightFail::ControlMtuTooSmall(m) => format!(
            "MTU sortante du canal de contrôle L2CAP = {m}, sous {} : une trame de 66 octets ne passerait pas en un morceau (hidp Linux ne fragmente pas ; la session HID serait coupée)",
            devname::MIN_CONTROL_MTU
        ),
    }
}

fn preflight_text(p: &Preflight, lang: Lang, dry_run: bool) -> String {
    let fails: Vec<_> = devname::preflight(p)
        .into_iter()
        .filter(|f| !(dry_run && f.is_mtu()))
        .collect();
    let mut s = String::new();
    if fails.is_empty() {
        s.push_str(lang.t("Pré-vol : ok", "Pre-flight: ok"));
        s.push_str(&format!(
            " ({}{}{}{})\n",
            lang.t("doctor vert, ", "doctor green, "),
            match p.battery_pct {
                Some(b) => format!("{} {b:.0} %, ", lang.t("batterie", "battery")),
                None => String::new(),
            },
            lang.t("disjoncteur fermé", "breaker closed"),
            match p.control_mtu {
                Some(m) if !dry_run => format!(
                    ", {} {m} >= {}",
                    lang.t("MTU sortante", "outgoing MTU"),
                    devname::MIN_CONTROL_MTU
                ),
                _ => String::new(),
            }
        ));
    } else {
        for f in &fails {
            s.push_str(&format!(
                "{} {}\n",
                lang.t("Pré-vol : ÉCHEC -", "Pre-flight: FAILED -"),
                fail_text(f, lang)
            ));
        }
    }
    if dry_run {
        s.push_str(&format!(
            "{} {} {}\n",
            lang.t(
                "Pré-vol : la MTU du canal de contrôle n'est pas lue en essai à blanc (elle demande pkexec) ; lue et exigée >=",
                "Pre-flight: control-channel MTU not probed in a dry run (needs pkexec); probed and required >="
            ),
            devname::MIN_CONTROL_MTU,
            lang.t("avec --check ou --write-device-name", "with --check or --write-device-name")
        ));
    }
    s
}

fn restore_command(backup: &Path) -> String {
    format!(
        "akmctl rename --device-name --restore {} --write-device-name",
        backup.display()
    )
}

#[allow(clippy::too_many_arguments)]
fn rename(
    name: &str,
    mode: Mode,
    mac: Option<String>,
    config_allowed: bool,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    io.out(lang.t(TWO_NAMES_FR, TWO_NAMES));
    let frames = match devname::prepare(name) {
        Ok(f) => f,
        Err(e) => {
            io.journal(&format!(
                "akmctl: {} {e}",
                lang.t("nom refusé :", "name refused:")
            ));
            return EXIT_ERROR;
        }
    };
    io.out(&format!(
        "\n{} {name:?}\n{}\n{}",
        lang.t("Nouveau nom stocké dans le clavier :", "New name stored in the keyboard:"),
        lang.t(
            "Trame envoyée par le renommage d'Apple (Lion 10.7.5 setDeviceName:, établie par désassemblage) :",
            "Frame Apple's rename sends (Lion 10.7.5 setDeviceName:, established by disassembly):"
        ),
        devname::render_frames(&frames)
    ));
    match mode {
        Mode::DryRun => dry_run(name, mac, config_allowed, lang, world, io),
        Mode::Check | Mode::Write => guarded(
            &Request::Rename(name.to_string()),
            frames,
            mode,
            mac,
            config_allowed,
            lang,
            world,
            io,
        ),
    }
}

fn dry_run(
    _name: &str,
    mac: Option<String>,
    config_allowed: bool,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    io.out(lang.t(
        "\nESSAI À BLANC : rien n'est écrit dans le clavier.\n",
        "\nDRY RUN: nothing is written to the keyboard.\n",
    ));
    let interactive = io.interactive();
    match world.snapshot() {
        Ok(s) => {
            let green = world.doctor_green(mac.as_deref());
            io.out(&preflight_text(
                &preflight_from(&s, green, interactive, None),
                lang,
                true,
            ));
            match cached_raw(&s) {
                Some(raw) => {
                    let m = snapshot_mac(&s).unwrap_or_else(|| "unknown".into());
                    let now = world.now_unix();
                    match Backup::new(&m, &raw, now, "daemon-cache")
                        .map_err(|e| e.to_string())
                        .and_then(|b| world.save_backup(&b).map_err(|e| e.to_string()))
                    {
                        Ok(p) => io.out(&format!(
                            "{} {} (0600)\n",
                            lang.t("Sauvegarde du nom actuel :", "Backup of the current name:"),
                            p.display()
                        )),
                        Err(e) => io.out(&format!("{} {e}\n", lang.t("Sauvegarde : ÉCHEC -", "Backup: FAILED -"))),
                    }
                }
                None => io.out(lang.t(
                    "Sauvegarde : pas encore possible (le démon n'a pas lu 0x51-0x54 dans cette connexion)\n",
                    "Backup: not possible yet (the daemon has not read 0x51-0x54 in this connection)\n",
                )),
            }
        }
        Err((_, m)) => io.out(&format!(
            "{} ({m})\n",
            lang.t(
                "Pré-vol : démon indisponible",
                "Pre-flight: daemon unavailable"
            )
        )),
    }
    io.out(&proof_state(lang));
    io.out(&locks_state(lang, config_allowed, interactive));
    io.out(lang.t(
        "\nÉtape suivante sans rien écrire : la même commande avec --check (pré-vol complet, MTU lue par pkexec). L'écriture réelle : --write-device-name et les trois verrous ci-dessus.\n",
        "\nNext step without writing: the same command with --check (whole pre-flight, MTU read via pkexec). The real write: --write-device-name and the three locks above.\n",
    ));
    EXIT_OK
}

#[allow(clippy::too_many_arguments)]
fn restore(
    file: &Path,
    mode: Mode,
    mac: Option<String>,
    config_allowed: bool,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    let b = match devname::read_backup(file) {
        Ok(b) => b,
        Err(e) => {
            io.journal(&format!(
                "akmctl: {} {e}",
                lang.t("sauvegarde refusée :", "backup refused:")
            ));
            return EXIT_ERROR;
        }
    };
    io.out(&format!(
        "{} {}: {} {:?}, {} {}, {} {}\n",
        lang.t("Sauvegarde", "Backup"),
        file.display(),
        lang.t("nom", "name"),
        b.name,
        lang.t("clavier", "keyboard"),
        b.mac,
        lang.t("enregistrée le", "saved at"),
        devname::utc_stamp(b.created_unix)
    ));
    let frames = match devname::frames_for_restore(&b) {
        Ok(f) => f,
        Err(e) => {
            io.journal(&format!(
                "akmctl: {} {e}",
                lang.t("sauvegarde refusée :", "backup refused:")
            ));
            return EXIT_ERROR;
        }
    };
    io.out(&devname::render_frames(&frames));
    if mode == Mode::DryRun {
        io.out(lang.t(
            "ESSAI À BLANC : rien d'écrit. Ajoutez --check (pré-vol complet) ou --write-device-name pour restaurer (même protocole, mêmes trois verrous, nouvelle confirmation).\n",
            "DRY RUN: nothing written. Add --check (whole pre-flight) or --write-device-name to restore (same protocol, same three locks, new confirmation).\n",
        ));
        io.out(&proof_state(lang));
        io.out(&locks_state(lang, config_allowed, io.interactive()));
        return EXIT_OK;
    }
    guarded(
        &Request::Restore(b),
        frames,
        mode,
        mac,
        config_allowed,
        lang,
        world,
        io,
    )
}

// ── the guarded flow ───────────────────────────────────────────────────────

/// How lock 1 is (or is not) lifted in this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Consent {
    /// `[apple] allow_device_name_write = true`.
    Config,
    /// To be typed (`ECRIRE`) after the plan, this run only.
    Interactive,
    /// `--check`: never asked, never written.
    Check,
}

/// The real environment of [`devname::run`]: daemon snapshot, doctor,
/// terminal, MTU probe (one `pkexec` per command), backup dir, hidraw door,
/// and the plan/consent/countdown shown to the user.
struct Flow<'a> {
    world: &'a mut dyn World,
    io: &'a mut dyn Io,
    lang: Lang,
    consent: Consent,
    mac: Option<String>,
    frames: Vec<Frame>,
    door: Option<Box<dyn FeatureSink>>,
    /// Result of the single MTU probe of this command (`None` = not run yet).
    mtu: Option<Result<u16, String>>,
    probes: u32,
    last_preflight: Option<Preflight>,
    backup_path: Option<PathBuf>,
    consent_typed: bool,
    /// Why the hidraw door did not open (then nothing can be written).
    door_error: Option<String>,
}

/// A sink that refuses: used when the door is not open.
struct Closed;

impl FeatureSink for Closed {
    fn set_feature(&self, _op: akm_core::registry::WriteOp, _report: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "no hidraw write door: nothing written",
        ))
    }
}

static CLOSED: Closed = Closed;

impl Flow<'_> {
    fn snapshot(&mut self) -> Option<Snapshot> {
        match self.world.snapshot() {
            Ok(s) => Some(s),
            Err((_, m)) => {
                self.io
                    .journal(&format!("[devname] daemon unavailable: {m}"));
                None
            }
        }
    }

    /// Lock 2: the outgoing MTU of the control channel, probed once (the
    /// second pre-flight call, just before the write, reuses the value).
    fn control_mtu(&mut self, snapshot_mac: Option<&str>) -> Option<u16> {
        if self.mtu.is_none() {
            let Some(mac) = self
                .mac
                .clone()
                .or_else(|| snapshot_mac.map(str::to_string))
            else {
                self.io
                    .journal("[devname] MTU probe skipped: the keyboard's MAC is unknown");
                self.mtu = Some(Err("MAC unknown".into()));
                return None;
            };
            self.io.out(&format!(
                "\n{}\n",
                self.lang.t(
                    &format!("→ Verrou 2 : lecture de la MTU sortante du canal de contrôle L2CAP vers {mac} (pkexec akm-hid-control inspect --mac {mac} : UNE authentification administrateur, lecture seule, rien n'est envoyé)…"),
                    &format!("→ Lock 2: reading the outgoing MTU of the L2CAP control channel to {mac} (pkexec akm-hid-control inspect --mac {mac}: ONE administrator authentication, read-only, nothing sent)...")
                )
            ));
            self.probes += 1;
            let r = match self.world.probe_control_mtu(&mac) {
                Ok((m, text)) => {
                    for l in text
                        .lines()
                        .filter(|l| l.contains(" fd ") || l.contains("inspected"))
                    {
                        self.io.journal(&format!("[devname]   {}", l.trim()));
                    }
                    self.io.journal(&format!(
                        "[devname] control-channel outgoing MTU = {m} (required >= {})",
                        devname::MIN_CONTROL_MTU
                    ));
                    Ok(m)
                }
                Err(e) => {
                    self.io
                        .journal(&format!("[devname] control-channel MTU unknown: {e}"));
                    Err(e)
                }
            };
            self.mtu = Some(r);
        }
        self.mtu.as_ref().and_then(|r| r.as_ref().ok().copied())
    }

    /// The plan: what will be written, where, the backup, the pre-flight,
    /// the risks. Shown once, just before the consent.
    fn plan(&self) -> String {
        let l = self.lang;
        let mut s = String::new();
        s.push_str(l.t(
            "\n━━━ PLAN ━━━\nCe qui va être écrit dans la MÉMOIRE du clavier (micrologiciel BCM2042, rapport Feature 0x55, UNE trame) :\n",
            "\n━━━ PLAN ━━━\nWhat will be written into the keyboard's MEMORY (BCM2042 firmware, Feature report 0x55, ONE frame):\n",
        ));
        for f in &self.frames {
            s.push_str(&format!(
                "  {} {:#04x} {} : {}\n  {} : {}\n",
                l.t("rapport", "report"),
                f.id(),
                l.t("(65 octets)", "(65 bytes)"),
                devname::hex(&f.report),
                l.t("sur le fil (66 octets)", "on the wire (66 bytes)"),
                devname::hex(&f.wire())
            ));
        }
        if let Some(p) = &self.last_preflight {
            s.push_str(&preflight_text(p, l, false));
        }
        match &self.backup_path {
            Some(p) => s.push_str(&format!(
                "{} {} (0600)\n  {} {}\n",
                l.t("Sauvegarde du nom actuel :", "Backup of the current name:"),
                p.display(),
                l.t("retour arrière :", "rollback:"),
                restore_command(p)
            )),
            None => s.push_str(l.t(
                "Sauvegarde : aucune nouvelle (retour arrière = réécriture d'une sauvegarde existante)\n",
                "Backup: none new (a rollback rewrites an existing backup)\n",
            )),
        }
        s.push_str(&risks(l));
        s.push_str(match self.consent {
            Consent::Config => l.t(
                "Verrou 1 : levé par config.toml ([apple] allow_device_name_write = true).\n",
                "Lock 1: lifted by config.toml ([apple] allow_device_name_write = true).\n",
            ),
            Consent::Interactive => l.t(
                "Verrou 1 : config.toml fermé (défaut). Le consentement vaut pour CETTE exécution seulement ; rien n'est écrit dans config.toml.\n",
                "Lock 1: config.toml closed (default). The consent is for THIS run only; nothing is written into config.toml.\n",
            ),
            Consent::Check => l.t(
                "Verrou 1 : non demandé (--check).\n",
                "Lock 1: not asked (--check).\n",
            ),
        });
        s
    }

    fn read_trimmed(&mut self) -> Option<String> {
        self.io
            .read_line()
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
    }
}

impl RenameEnv for Flow<'_> {
    fn preflight(&mut self) -> Preflight {
        let interactive = self.io.interactive();
        let p = match self.snapshot() {
            Some(s) => {
                let mtu = self.control_mtu(snapshot_mac(&s).as_deref());
                let mac = self.mac.clone();
                let green = self.world.doctor_green(mac.as_deref());
                preflight_from(&s, green, interactive, mtu)
            }
            None => Preflight::default(),
        };
        if self.last_preflight.is_none() {
            self.last_preflight = Some(p.clone());
        }
        p
    }
    fn cached_raw(&mut self) -> Option<Vec<u8>> {
        self.snapshot().as_ref().and_then(cached_raw)
    }
    fn mac(&mut self) -> String {
        self.mac
            .clone()
            .or_else(|| self.snapshot().and_then(|s| snapshot_mac(&s)))
            .unwrap_or_else(|| "unknown".into())
    }
    fn now_unix(&mut self) -> u64 {
        self.world.now_unix()
    }
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
        let p = self.world.save_backup(b)?;
        self.backup_path = Some(p.clone());
        Ok(p)
    }
    fn confirm(&mut self, prompt: &str, expected: &str) -> bool {
        let l = self.lang;
        let plan = self.plan();
        self.io.out(&plan);
        if self.consent == Consent::Check {
            self.io.out(l.t(
                "\n--check : arrêt ici, avant toute demande de consentement. Rien n'est écrit, rien n'est demandé.\n",
                "\n--check: stopping here, before any consent is asked. Nothing is written, nothing is asked.\n",
            ));
            return false;
        }
        if !self.io.interactive() {
            self.io.out(l.t(
                "\nrefusé : confirmation interactive requise (stdin et stdout doivent être un terminal)\n",
                "\nrefused: interactive confirmation required (stdin and stdout must be a terminal)\n",
            ));
            return false;
        }
        if self.consent == Consent::Interactive {
            self.io.out(l.t(
                &format!("\nPour consentir à cette écriture, pour cette exécution seulement, tapez exactement {CONSENT_WORD} (majuscules, sans accent) puis Entrée ; tout autre texte annule : "),
                &format!("\nTo consent to this write, for this run only, type exactly {CONSENT_WORD} (upper case) then Enter; anything else cancels: "),
            ));
            if self.read_trimmed().as_deref() != Some(CONSENT_WORD) {
                self.io.out(l.t(
                    &format!("\nannulé : {CONSENT_WORD} n'a pas été tapé exactement ; rien n'est écrit (verrou 1).\n"),
                    &format!("\ncancelled: {CONSENT_WORD} was not typed exactly; nothing written (lock 1).\n"),
                ));
                self.io.journal("[devname] decision: cancelled (lock 1, interactive consent not typed); nothing written, config.toml untouched");
                return false;
            }
            self.consent_typed = true;
            self.io.journal(&format!(
                "[devname] lock 1 lifted for THIS run only: {CONSENT_WORD} typed in the terminal (config.toml untouched)"
            ));
        }
        self.io.journal(&format!("[devname] {}", prompt.trim_end()));
        self.io.out(l.t(
            &format!("\nVerrou 3 : retapez le nom exactement ({expected:?}) puis Entrée ; tout autre texte annule : "),
            &format!("\nLock 3: type the name again exactly ({expected:?}) then Enter; anything else cancels: "),
        ));
        let ok = self.read_trimmed().as_deref() == Some(expected);
        if !ok {
            self.io.out(l.t(
                "\nannulé : le nom retapé diffère ; rien n'est écrit (verrou 3).\n",
                "\ncancelled: the name typed again differs; nothing written (lock 3).\n",
            ));
            return false;
        }
        // Only now is the hardware node opened (under the HID lock).
        match self.world.open_door() {
            Ok(d) => self.door = Some(d),
            Err(e) => {
                self.io
                    .journal(&format!("[devname] write door not opened: {e}"));
                self.door_error = Some(e);
            }
        }
        true
    }
    fn sink(&self) -> &dyn FeatureSink {
        match &self.door {
            Some(d) => d.as_ref(),
            None => &CLOSED,
        }
    }
    fn wait_reconnect(&mut self, max: Duration) -> Reconnect {
        // Release the HID lock first: the daemon must read the name again.
        self.door = None;
        let l = self.lang;
        let total = max.as_secs();
        self.io.out(l.t(
            &format!("\n✎ Trame envoyée.\n→ ÉTEIGNEZ le clavier (bouton latéral 3 s, voyant éteint), ATTENDEZ 5 s, RALLUMEZ-le.\n  Le nom ne se relit qu'après une reconnexion. Attente au plus {total} s…\n"),
            &format!("\n✎ Frame sent.\n→ Switch the keyboard OFF (side button, 3 s, light off), WAIT 5 s, switch it ON.\n  The name is only read back after a reconnection. Waiting at most {total} s...\n"),
        ));
        let mut seen_down = false;
        for elapsed in 0..total {
            if let Ok(s) = self.world.snapshot() {
                if !s.connected {
                    if !seen_down {
                        self.io.out(l.t(
                            "\r  clavier éteint, rallumez-le…      ",
                            "\r  keyboard off, switch it on...      ",
                        ));
                    }
                    seen_down = true;
                } else if seen_down {
                    if let Some(raw) = cached_raw(&s) {
                        self.io.out(&format!(
                            "\r  {} ({elapsed} s)                     \n",
                            l.t("reconnecté, nom relu", "reconnected, name read back")
                        ));
                        // BlueZ's Device1.Name (what Apple would check through
                        // the HCI remote name), informative only.
                        let bluez_name = s.keyboard.as_ref().and_then(|k| k.device.name.clone());
                        return Reconnect::Back { raw, bluez_name };
                    }
                }
            }
            self.io.out(&format!(
                "\r  {} {} s   ",
                l.t("restant :", "left:"),
                total - elapsed
            ));
            self.world.sleep(Duration::from_secs(1));
        }
        self.io.out("\n");
        Reconnect::Timeout
    }
    fn log(&mut self, line: &str) {
        // akm-core knows one lock 1 (the boolean); say how it was lifted.
        let line = match self.consent {
            Consent::Interactive if line.starts_with("[devname] lock 1 lifted") => {
                "[devname] lock 1: config.toml closed; the consent (ECRIRE) will be asked in this terminal for this run only, after the pre-flight, the backup and the plan".to_string()
            }
            Consent::Check if line.starts_with("[devname] lock 1 lifted") => {
                "[devname] --check: lock 1 not evaluated, nothing will be asked nor written".to_string()
            }
            Consent::Check if line.contains("cancelled (lock 3") => {
                "[devname] --check: stopped before any consent; nothing written".to_string()
            }
            _ => line.to_string(),
        };
        self.io.journal(&line);
    }
}

#[allow(clippy::too_many_arguments)]
fn guarded(
    req: &Request,
    frames: Vec<Frame>,
    mode: Mode,
    mac: Option<String>,
    config_allowed: bool,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    let interactive = io.interactive();
    let consent = match mode {
        Mode::Check => Consent::Check,
        _ if config_allowed => Consent::Config,
        _ => Consent::Interactive,
    };
    // Lock 1 as akm-core sees it: lifted by the configuration (automation),
    // or lifted in principle by a terminal where ECRIRE will be demanded
    // before anything is written; --check never writes (no consent asked, no
    // door). Without a terminal and without the key: refused, as before.
    let allow_write = match consent {
        Consent::Config => true,
        Consent::Interactive => interactive,
        Consent::Check => true,
    };
    io.out(match mode {
        Mode::Check => lang.t(
            "\n--check : pré-vol complet (MTU lue par pkexec), sauvegarde et plan ; RIEN n'est écrit, rien n'est demandé.\n",
            "\n--check: whole pre-flight (MTU read via pkexec), backup and plan; NOTHING is written, nothing is asked.\n",
        ),
        _ => lang.t(
            "\nÉCRITURE RÉELLE demandée. Ordre : pré-vol (doctor, batterie, disjoncteur, MTU par pkexec) → plan et sauvegarde → consentement → nom retapé → UNE écriture → reconnexion → relecture.\n",
            "\nREAL WRITE requested. Order: pre-flight (doctor, battery, breaker, MTU via pkexec) → plan and backup → consent → name typed again → ONE write → reconnection → read back.\n",
        ),
    });
    let mut flow = Flow {
        world,
        io,
        lang,
        consent,
        mac,
        frames,
        door: None,
        mtu: None,
        probes: 0,
        last_preflight: None,
        backup_path: None,
        consent_typed: false,
        door_error: None,
    };
    let mut session = WriteSession::new();
    let o = devname::run(
        req,
        devname::SEQUENCE_PROOF,
        allow_write,
        &mut session,
        &mut flow,
    );
    debug_assert!(flow.probes <= 1, "one pkexec per command");
    if let (Outcome::WriteFailed(_), Some(d)) = (&o, &flow.door_error) {
        // The door never opened: the closed sink refused, nothing left akmctl.
        flow.io.out(lang.t(
            &format!("\nArrêt : le nœud hidraw ne s'est pas ouvert ({d}). Rien n'a été écrit.\n"),
            &format!("\nStopped: the hidraw node did not open ({d}). Nothing was written.\n"),
        ));
        return EXIT_ERROR;
    }
    let text = report(&o, mode, lang, interactive);
    flow.io.out(&text.0);
    text.1
}

/// Text for the user and exit code of an outcome.
pub fn report(o: &Outcome, mode: Mode, lang: Lang, interactive: bool) -> (String, u8) {
    let l = lang;
    match o {
        Outcome::NotProven => (
            format!(
                "{}\n{}",
                l.t(
                    "\nREFUSÉ (NotProven) : les octets exacts qu'Apple envoie ne sont pas établis ; rien n'a été touché (ni sauvegarde, ni confirmation, ni écriture).",
                    "\nREFUSED (NotProven): the exact bytes Apple sends to rename this keyboard are not established; nothing was touched (no backup, no confirmation, no write)."
                ),
                proof_state(l)
            ),
            EXIT_ERROR,
        ),
        Outcome::ConfigDisabled => (
            format!(
                "{}\n\n{}\n\n{}\n",
                l.t(
                    "\nREFUSÉ (verrou 1, consentement) : [apple] allow_device_name_write n'est pas true et cette commande ne tourne pas dans un terminal interactif ; rien n'a été touché (ni pré-vol, ni pkexec, ni sauvegarde, ni confirmation, ni écriture).\nDeux voies :\n  * dans un TERMINAL (konsole…), relancer la même commande : le consentement se tape (ECRIRE) pour cette exécution seulement, rien n'est écrit dans config.toml ;\n  * automatisation (sans terminal) : ajouter à ~/.config/apple-kb-monitor/config.toml",
                    "\nREFUSED (lock 1, consent): [apple] allow_device_name_write is not true and this command does not run in an interactive terminal; nothing was touched (no pre-flight, no pkexec, no backup, no confirmation, no write).\nTwo ways:\n  * in a TERMINAL (konsole...), run the same command again: the consent is typed (ECRIRE) for that run only, nothing is written into config.toml;\n  * automation (no terminal): add to ~/.config/apple-kb-monitor/config.toml"
                ),
                devname::CONFIG_HOWTO,
                l.t(
                    "Lire d'abord docs/RENOMMER-CLAVIER.md §5.3 et §6 : le HANDSHAKE du micrologiciel au SET 0x55 (U5) et la persistance au changement de piles (U3) ne sont PAS mesurés.",
                    "Read docs/RENOMMER-CLAVIER.md §5.3 and §6 first: the firmware's HANDSHAKE to SET 0x55 (U5) and persistence across a battery change (U3) are NOT measured."
                )
            ),
            EXIT_LOCK1,
        ),
        Outcome::Preflight(fails) => {
            let mut s = String::from(if fails.iter().any(|f| f.is_mtu()) {
                l.t(
                    "\nREFUSÉ (verrou 2, MTU du canal de contrôle) : rien n'a été écrit (ni sauvegarde, ni confirmation).",
                    "\nREFUSED (lock 2, control-channel MTU): nothing was written (no backup, no confirmation).",
                )
            } else {
                l.t(
                    "\nREFUSÉ (pré-vol) : rien n'a été écrit (ni sauvegarde, ni confirmation).",
                    "\nREFUSED (pre-flight): nothing was written (no backup, no confirmation).",
                )
            });
            s.push('\n');
            for f in fails {
                s.push_str(&format!("  - {}\n", fail_text(f, l)));
            }
            if fails.iter().any(|f| f.is_mtu()) {
                s.push_str(&format!(
                    "  {}\n",
                    l.t(
                        &format!("La MTU est lue sur la socket L2CAP vivante par `pkexec akm-hid-control inspect --mac <MAC>` (lecture seule). Si le helper ne l'a pas donnée, réinstaller le paquet ; si elle est sous {}, ce clavier ne peut pas recevoir la trame de 66 octets en un morceau depuis Linux.", devname::MIN_CONTROL_MTU),
                        &format!("The MTU is read on the live L2CAP socket by `pkexec akm-hid-control inspect --mac <MAC>` (read-only). If the helper did not report it, reinstall the package; if it is below {}, this keyboard cannot take the 66-byte frame in one piece from Linux.", devname::MIN_CONTROL_MTU)
                    )
                ));
            }
            if mode == Mode::Check {
                s.push_str(l.t(
                    "✗ --check : l'écriture serait refusée ici.\n",
                    "✗ --check: the write would be refused here.\n",
                ));
            }
            (s, EXIT_PREFLIGHT)
        }
        Outcome::Cancelled if mode == Mode::Check => (
            l.t(
                &format!("\n✓ --check : tout le pré-vol est vert, la sauvegarde est faite, rien n'a été écrit. Pour écrire : la même commande avec --write-device-name (le consentement {CONSENT_WORD} et le nom retapé seront demandés{}).\n", if interactive { "" } else { " ; dans un terminal" }),
                &format!("\n✓ --check: the whole pre-flight is green, the backup is made, nothing was written. To write: the same command with --write-device-name (the consent {CONSENT_WORD} and the name typed again will be asked{}).\n", if interactive { "" } else { "; in a terminal" }),
            ).to_string(),
            EXIT_OK,
        ),
        Outcome::Cancelled => (
            l.t(
                "\nAnnulé au clavier : rien n'a été écrit dans le clavier, rien dans config.toml. La sauvegarde reste dans ~/.local/state/apple-kb-monitor/.\n",
                "\nCancelled at the keyboard: nothing was written into the keyboard, nothing into config.toml. The backup stays in ~/.local/state/apple-kb-monitor/.\n",
            )
            .to_string(),
            EXIT_CANCELLED,
        ),
        Outcome::Verified { backup, bluez_name } => {
            let mut s = String::from(l.t(
                "\n✓ Nom écrit et relu identique (0x51-0x54).\n",
                "\n✓ Name written and read back identical (0x51-0x54).\n",
            ));
            if let Some(n) = bluez_name {
                s.push_str(&format!(
                    "  {} {n:?} {}\n",
                    l.t("BlueZ Device1.Name maintenant :", "BlueZ Device1.Name now:"),
                    l.t(
                        "(informatif ; BlueZ peut garder son nom en cache jusqu'à sa prochaine requête de nom)",
                        "(informative; BlueZ may still show its cached name until its next remote name request)"
                    )
                ));
            }
            if let Some(b) = backup {
                s.push_str(&format!(
                    "  {} {}\n",
                    l.t("sauvegarde conservée :", "backup kept:"),
                    b.display()
                ));
            }
            s.push_str(l.t(
                "  Vérifiez sur un autre appareil ou après un changement de piles (risque U3 non mesuré).\n",
                "  Check on another device or after a battery change (risk U3 not measured).\n",
            ));
            (s, EXIT_OK)
        }
        Outcome::Mismatch { read, backup, bluez_name } => {
            let mut s = format!(
                "\n✗ {} {}\n",
                l.t("Le nom relu diffère :", "The name read back differs:"),
                devname::hex(read)
            );
            if let Some(n) = bluez_name {
                s.push_str(&format!("  {} {n:?}\n", l.t("BlueZ Device1.Name maintenant :", "BlueZ Device1.Name now:")));
            }
            match backup {
                Some(b) => s.push_str(&format!(
                    "  {}\n  {}\n",
                    l.t(
                        "RETOUR ARRIÈRE (même protocole, mêmes trois verrous, nouvelle confirmation, nouvelle session) : tapez exactement",
                        "ROLLBACK (same protocol, same three locks, new confirmation, new session): type exactly"
                    ),
                    restore_command(b)
                )),
                None => s.push_str(l.t(
                    "  Retour arrière : relancer la commande --restore avec la sauvegarde d'origine.\n",
                    "  Rollback: run the --restore command again with the original backup.\n",
                )),
            }
            (s, EXIT_MISMATCH)
        }
        Outcome::NoReconnect => (
            l.t(
                "\n✗ Le clavier n'est pas revenu dans le délai. Rien d'autre n'est tenté. Éteignez-le et rallumez-le, puis `akmctl rename --device-name --show` ; si le nom diffère, retour arrière : la commande --restore affichée dans le plan.\n",
                "\n✗ The keyboard did not come back in time. Nothing else is attempted. Switch it off and on, then `akmctl rename --device-name --show`; if the name differs, rollback: the --restore command shown in the plan.\n",
            )
            .to_string(),
            EXIT_NO_RECONNECT,
        ),
        other => (
            format!(
                "\n{} {other:?}. {}\n",
                l.t("Arrêt :", "Stopped:"),
                if other.wrote() {
                    l.t("Une trame peut être partie.", "A frame may have been sent.")
                } else {
                    l.t("Rien n'a été écrit.", "Nothing was written.")
                }
            ),
            EXIT_ERROR,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::registry::WriteOp;
    use akm_core::KbReport;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    const MAC: &str = "04:DB:56:CA:42:EE";
    /// "Clavier de maria #1", the 4 × 8 bytes of 0x51-0x54 as the daemon caches them.
    const OLD_HEX: &str = concat!(
        "436c617669657220",
        "6465206d61726961",
        "2023310000000000",
        "0000000000000000"
    );

    fn fixture() -> serde_json::Value {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/devname/lion_setdevicename_frames.json");
        serde_json::from_str(
            &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
        )
        .unwrap()
    }

    fn snap(connected: bool, raw_hex: Option<&str>) -> Snapshot {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(99.0);
        k.device.mac = Some(MAC.into());
        k.device.name = Some("Clavier de maria #1".into());
        k.device.name_on_keyboard_hex = raw_hex.map(str::to_string);
        Snapshot {
            connected,
            keyboard: Some(k),
            ..Default::default()
        }
    }

    /// A terminal with scripted lines.
    struct FakeIo {
        tty: bool,
        lines: VecDeque<String>,
        out: String,
        journal: Vec<String>,
    }

    impl FakeIo {
        fn new(tty: bool, lines: &[&str]) -> Self {
            Self {
                tty,
                lines: lines.iter().map(|l| format!("{l}\n")).collect(),
                out: String::new(),
                journal: Vec::new(),
            }
        }
    }

    impl Io for FakeIo {
        fn interactive(&self) -> bool {
            self.tty
        }
        fn read_line(&mut self) -> Option<String> {
            self.lines.pop_front()
        }
        fn out(&mut self, s: &str) {
            self.out.push_str(s);
        }
        fn journal(&mut self, s: &str) {
            self.journal.push(s.to_string());
        }
    }

    type Writes = Rc<RefCell<Vec<(WriteOp, Vec<u8>)>>>;

    /// The spy sink the fake door hands out.
    #[derive(Default)]
    struct Spy {
        writes: Writes,
        events: Rc<RefCell<Vec<String>>>,
    }

    impl FeatureSink for Spy {
        fn set_feature(&self, op: WriteOp, report: &[u8]) -> std::io::Result<()> {
            self.writes.borrow_mut().push((op, report.to_vec()));
            self.events
                .borrow_mut()
                .push(format!("write {:#04x}", report[0]));
            Ok(())
        }
    }

    struct FakeWorld {
        connected: bool,
        raw_hex: Option<String>,
        doctor: bool,
        mtu: Result<(u16, String), String>,
        probes: u32,
        door_fails: bool,
        writes: Writes,
        events: Rc<RefCell<Vec<String>>>,
        backups: Vec<Backup>,
        /// Applied one per `sleep`, during the reconnection wait.
        script: VecDeque<(bool, Option<String>)>,
        sleeps: u32,
    }

    impl FakeWorld {
        fn green() -> Self {
            Self {
                connected: true,
                raw_hex: Some(OLD_HEX.into()),
                doctor: true,
                mtu: Ok((672, format!("akm-hid-control:   {MAC} fd 23: psm local 0x0000 peer 0x0011 cid 0x0041 state connected (hci handle 0x000b) mtu out 672 in 672\nakm-hid-control: {MAC}: inspected, nothing sent"))),
                probes: 0,
                door_fails: false,
                writes: Rc::default(),
                events: Rc::default(),
                backups: Vec::new(),
                script: VecDeque::new(),
                sleeps: 0,
            }
        }
        /// Off after 2 s, back with `new_hex` after 7 s.
        fn comes_back(mut self, new_hex: &str) -> Self {
            self.script = VecDeque::from(vec![
                (true, self.raw_hex.clone()),
                (false, None),
                (false, None),
                (false, None),
                (true, None),
                (true, None),
                (true, Some(new_hex.into())),
            ]);
            self
        }
    }

    impl World for FakeWorld {
        fn snapshot(&mut self) -> Result<Snapshot, (u8, String)> {
            Ok(snap(self.connected, self.raw_hex.as_deref()))
        }
        fn doctor_green(&mut self, mac: Option<&str>) -> bool {
            assert!(mac.is_none() || mac == Some(MAC));
            self.doctor
        }
        fn probe_control_mtu(&mut self, mac: &str) -> Result<(u16, String), String> {
            assert_eq!(mac, MAC);
            self.probes += 1;
            self.events.borrow_mut().push("pkexec".into());
            self.mtu.clone()
        }
        fn open_door(&mut self) -> Result<Box<dyn FeatureSink>, String> {
            self.events.borrow_mut().push("door".into());
            if self.door_fails {
                return Err("no Apple keyboard found (hidraw)".into());
            }
            Ok(Box::new(Spy {
                writes: self.writes.clone(),
                events: self.events.clone(),
            }))
        }
        fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
            self.events.borrow_mut().push("backup".into());
            self.backups.push(b.clone());
            Ok(PathBuf::from("/sim/devname-backup-20260930T235959Z.json"))
        }
        fn now_unix(&mut self) -> u64 {
            1_790_812_799
        }
        fn sleep(&mut self, d: Duration) {
            assert_eq!(d, Duration::from_secs(1));
            self.sleeps += 1;
            if let Some((c, r)) = self.script.pop_front() {
                self.connected = c;
                self.raw_hex = r;
            }
        }
    }

    fn write(name: &str, cfg: bool, w: &mut FakeWorld, io: &mut FakeIo) -> u8 {
        dispatch(
            Action::Rename {
                name: name.into(),
                mode: Mode::Write,
            },
            None,
            cfg,
            Lang::En,
            w,
            io,
        )
    }

    #[test]
    fn preflight_from_a_snapshot() {
        let s = snap(true, Some(OLD_HEX));
        let p = preflight_from(&s, true, true, Some(672));
        assert!(devname::preflight(&p).is_empty(), "{p:?}");
        assert_eq!(
            devname::name_from_raw(&cached_raw(&s).unwrap()).unwrap(),
            "Clavier de maria #1"
        );
        // MTU unknown or too small is a pre-flight failure of its own.
        let p = preflight_from(&s, true, true, None);
        assert_eq!(
            devname::preflight(&p),
            vec![PreflightFail::ControlMtuUnknown]
        );
        let p = preflight_from(&s, true, true, Some(48));
        assert_eq!(
            devname::preflight(&p),
            vec![PreflightFail::ControlMtuTooSmall(48)]
        );
        let mut s2 = s.clone();
        s2.keyboard.as_mut().unwrap().breaker_open = true;
        s2.keyboard.as_mut().unwrap().incomplete = true;
        let f = devname::preflight(&preflight_from(&s2, false, false, Some(672)));
        assert_eq!(f.len(), 4);
        assert!(cached_raw(&Snapshot::default()).is_none());
        assert_eq!(snapshot_mac(&s).as_deref(), Some(MAC));
    }

    #[test]
    fn language_follows_the_locale() {
        let env = |vals: &[(&str, &str)]| {
            let v: Vec<(String, String)> = vals
                .iter()
                .map(|(k, x)| (k.to_string(), x.to_string()))
                .collect();
            Lang::from_env(move |k| v.iter().find(|(kk, _)| kk == k).map(|(_, x)| x.clone()))
        };
        assert_eq!(env(&[("LANG", "fr_FR.UTF-8")]), Lang::Fr);
        assert_eq!(env(&[("LANG", "en_US.UTF-8")]), Lang::En);
        assert_eq!(env(&[("LC_ALL", "C"), ("LANG", "fr_FR.UTF-8")]), Lang::En);
        assert_eq!(env(&[("LC_ALL", ""), ("LC_MESSAGES", "fr_CA")]), Lang::Fr);
        assert_eq!(env(&[]), Lang::En);
    }

    #[test]
    fn no_tty_and_no_config_key_refuses_before_any_pre_flight_or_pkexec() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(false, &["ECRIRE", "Bureau"]);
        assert_eq!(write("Bureau", false, &mut w, &mut io), EXIT_LOCK1);
        assert_eq!(w.probes, 0, "no pkexec");
        assert!(w.events.borrow().is_empty() && w.writes.borrow().is_empty());
        assert!(io.out.contains("REFUSED (lock 1, consent)") && io.out.contains("TERMINAL"));
        assert!(io.out.contains("allow_device_name_write = true"));
        // Lines piped in are never read as a consent.
        assert_eq!(io.lines.len(), 2);
    }

    #[test]
    fn no_tty_with_the_config_key_is_refused_by_the_pre_flight_as_before() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(false, &["Bureau"]);
        assert_eq!(write("Bureau", true, &mut w, &mut io), EXIT_PREFLIGHT);
        assert_eq!(w.probes, 1);
        assert_eq!(
            *w.events.borrow(),
            vec!["pkexec"],
            "no backup, no door, no write"
        );
        assert!(io.out.contains("not an interactive terminal"));
    }

    #[test]
    fn wrong_consent_word_writes_nothing_after_the_backup() {
        for typed in [
            "ecrire",
            "Ecrire",
            "ECRIRE ",
            " ECRIRE",
            "OUI",
            "",
            "ECRIRE Bureau",
        ] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(true, &[typed, "Bureau"]);
            assert_eq!(
                write("Bureau", false, &mut w, &mut io),
                EXIT_CANCELLED,
                "{typed:?}"
            );
            assert_eq!(
                *w.events.borrow(),
                vec!["pkexec", "backup"],
                "{typed:?}: no door, no write"
            );
            assert!(w.writes.borrow().is_empty());
            assert!(
                io.out.contains("cancelled: ECRIRE was not typed exactly"),
                "{typed:?}"
            );
            assert!(io
                .journal
                .iter()
                .any(|l| l.contains("lock 1, interactive consent not typed")));
            // The plan was shown first: bytes, backup, rollback, risks.
            assert!(io.out.contains("PLAN") && io.out.contains("55 42 75 72 65 61 75"));
            assert!(io.out.contains(
                "--restore /sim/devname-backup-20260930T235959Z.json --write-device-name"
            ));
            assert!(io.out.contains("U5") && io.out.contains("U3"));
            assert!(io.out.contains("THIS run only"));
            // The name was never asked: the second line is still there.
            assert_eq!(io.lines.len(), 1, "{typed:?}");
        }
    }

    #[test]
    fn consent_then_a_different_name_writes_nothing() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["ECRIRE", "bureau"]);
        assert_eq!(write("Bureau", false, &mut w, &mut io), EXIT_CANCELLED);
        assert_eq!(*w.events.borrow(), vec!["pkexec", "backup"]);
        assert!(w.writes.borrow().is_empty());
        assert!(io.out.contains("the name typed again differs"));
        assert!(io
            .journal
            .iter()
            .any(|l| l.contains("lock 1 lifted for THIS run only")));
        // End of input at the name prompt cancels too.
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["ECRIRE"]);
        assert_eq!(write("Bureau", false, &mut w, &mut io), EXIT_CANCELLED);
        assert!(w.writes.borrow().is_empty());
    }

    #[test]
    fn interactive_consent_sends_the_fixture_frame_once_after_the_backup_with_one_pkexec() {
        let fx = fixture();
        for ex in fx["examples"].as_array().unwrap() {
            let name = ex["name"].as_str().unwrap();
            let want = devname::unhex(ex["report_hex"].as_str().unwrap()).unwrap();
            let back = ex["readback_0x51_0x54_expected_hex"].as_str().unwrap();
            let mut w = FakeWorld::green().comes_back(back);
            let mut io = FakeIo::new(true, &["ECRIRE", name]);
            assert_eq!(write(name, false, &mut w, &mut io), EXIT_OK, "{name}");
            assert_eq!(
                *w.events.borrow(),
                vec!["pkexec", "backup", "door", "write 0x55"],
                "{name}: one probe, backup, then the single write"
            );
            assert_eq!(w.probes, 1, "{name}: exactly one pkexec");
            let wr = w.writes.borrow();
            assert_eq!(wr.len(), 1);
            assert_eq!(wr[0].0, WriteOp::DeviceName);
            assert_eq!(wr[0].1, want, "{name}: the fixture bytes, exactly");
            assert_eq!(wr[0].1.len(), 65);
            assert_eq!(w.backups[0].name, "Clavier de maria #1");
            assert!(io.out.contains("✓ Name written and read back identical"));
            assert!(io.out.contains("Switch the keyboard OFF") && io.out.contains("left:"));
            assert!(io.out.contains("reconnected, name read back"));
            assert!(io
                .journal
                .iter()
                .any(|l| l.contains("ECRIRE typed in the terminal (config.toml untouched)")));
            assert!(
                !io.journal
                    .iter()
                    .any(|l| l.contains("allow_device_name_write = true")),
                "{name}: never claims the config lifted it"
            );
            assert!(w.sleeps >= 6 && w.sleeps < 180);
        }
    }

    #[test]
    fn config_key_skips_the_typed_consent_but_not_the_name() {
        let back = &fixture()["examples"][0];
        let name = back["name"].as_str().unwrap();
        let mut w = FakeWorld::green()
            .comes_back(back["readback_0x51_0x54_expected_hex"].as_str().unwrap());
        // Only the name is typed: ECRIRE is not asked.
        let mut io = FakeIo::new(true, &[name]);
        assert_eq!(write(name, true, &mut w, &mut io), EXIT_OK);
        assert_eq!(w.writes.borrow().len(), 1);
        assert!(!io.out.contains("type exactly ECRIRE"));
        assert!(io.out.contains("lifted by config.toml"));
        // With the key, a wrong name still cancels.
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["ECRIRE"]);
        assert_eq!(write(name, true, &mut w, &mut io), EXIT_CANCELLED);
        assert!(w.writes.borrow().is_empty());
    }

    #[test]
    fn mismatch_prints_the_exact_rollback_and_timeout_stops() {
        let name = "Bureau";
        let mut w = FakeWorld::green().comes_back(OLD_HEX);
        let mut io = FakeIo::new(true, &["ECRIRE", name]);
        assert_eq!(write(name, false, &mut w, &mut io), EXIT_MISMATCH);
        assert_eq!(w.writes.borrow().len(), 1, "no retry");
        assert!(io.out.contains("✗ The name read back differs"));
        assert!(io.out.contains("ROLLBACK"));
        assert!(io.out.contains("akmctl rename --device-name --restore /sim/devname-backup-20260930T235959Z.json --write-device-name"));
        // Never comes back: 180 polls, then the distinct code.
        let mut w = FakeWorld::green();
        w.script = VecDeque::from(vec![(false, None)]);
        let mut io = FakeIo::new(true, &["ECRIRE", name]);
        assert_eq!(write(name, false, &mut w, &mut io), EXIT_NO_RECONNECT);
        assert_eq!(w.sleeps, 180);
        assert!(io.out.contains("did not come back in time"));
    }

    #[test]
    fn check_runs_the_whole_pre_flight_with_one_pkexec_and_never_asks_nor_writes() {
        let check = |w: &mut FakeWorld, io: &mut FakeIo, cfg: bool| {
            dispatch(
                Action::Rename {
                    name: "Bureau".into(),
                    mode: Mode::Check,
                },
                None,
                cfg,
                Lang::En,
                w,
                io,
            )
        };
        for (tty, cfg) in [(true, false), (true, true), (false, true)] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(tty, &["ECRIRE", "Bureau"]);
            let code = check(&mut w, &mut io, cfg);
            assert_eq!(w.probes, 1, "tty {tty}: exactly one pkexec");
            assert!(w.writes.borrow().is_empty() && !w.events.borrow().iter().any(|e| e == "door"));
            assert_eq!(io.lines.len(), 2, "nothing is read from the terminal");
            if tty {
                assert_eq!(code, EXIT_OK);
                assert_eq!(*w.events.borrow(), vec!["pkexec", "backup"]);
                assert!(
                    io.out.contains("✓ --check")
                        && io.out.contains("PLAN")
                        && io.out.contains("outgoing MTU 672 >= 66")
                );
            } else {
                // The missing terminal is a pre-flight fact: reported, not fatal to the check itself.
                assert_eq!(code, EXIT_PREFLIGHT);
                assert!(io.out.contains("would be refused here"));
            }
        }
        // MTU too small: refused at lock 2, no backup.
        let mut w = FakeWorld::green();
        w.mtu = Ok((48, String::new()));
        let mut io = FakeIo::new(true, &[]);
        assert_eq!(check(&mut w, &mut io, false), EXIT_PREFLIGHT);
        assert_eq!(*w.events.borrow(), vec!["pkexec"]);
        assert!(io.out.contains("lock 2") && io.out.contains("48"));
        // pkexec dismissed: unknown MTU, refused.
        let mut w = FakeWorld::green();
        w.mtu = Err("authentication dismissed or not authorized (pkexec 126)".into());
        let mut io = FakeIo::new(true, &[]);
        assert_eq!(check(&mut w, &mut io, false), EXIT_PREFLIGHT);
        assert!(io.journal.iter().any(|l| l.contains("pkexec 126")));
    }

    #[test]
    fn dry_run_never_probes_and_shows_the_locks() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["ECRIRE", "Bureau"]);
        let code = dispatch(
            Action::Rename {
                name: "Bureau".into(),
                mode: Mode::DryRun,
            },
            None,
            false,
            Lang::Fr,
            &mut w,
            &mut io,
        );
        assert_eq!(code, EXIT_OK);
        assert_eq!(w.probes, 0);
        assert_eq!(*w.events.borrow(), vec!["backup"]);
        assert_eq!(io.lines.len(), 2);
        assert!(
            io.out.contains("ESSAI À BLANC")
                && io.out.contains("Trois verrous")
                && io.out.contains("ECRIRE")
        );
        // A door that cannot open: nothing written, distinct stop.
        let mut w = FakeWorld::green();
        w.door_fails = true;
        let mut io = FakeIo::new(true, &["ECRIRE", "Bureau"]);
        assert_eq!(write("Bureau", false, &mut w, &mut io), EXIT_ERROR);
        assert!(w.writes.borrow().is_empty());
        assert!(io.out.contains("Nothing was written"));
    }

    #[test]
    fn french_flow_end_to_end() {
        let back = &fixture()["examples"][0];
        let name = back["name"].as_str().unwrap();
        let mut w = FakeWorld::green()
            .comes_back(back["readback_0x51_0x54_expected_hex"].as_str().unwrap());
        let mut io = FakeIo::new(true, &["ECRIRE", name]);
        let code = dispatch(
            Action::Rename {
                name: name.into(),
                mode: Mode::Write,
            },
            Some(MAC.into()),
            false,
            Lang::Fr,
            &mut w,
            &mut io,
        );
        assert_eq!(code, EXIT_OK);
        for s in [
            "ÉCRITURE RÉELLE demandée",
            "Verrou 2 : lecture de la MTU",
            "MÉMOIRE du clavier",
            "Sauvegarde du nom actuel",
            "RISQUES NON MESURÉS",
            "CETTE exécution seulement",
            "tapez exactement ECRIRE",
            "Verrou 3 : retapez le nom",
            "ÉTEIGNEZ le clavier",
            "ATTENDEZ 5 s",
            "restant :",
            "✓ Nom écrit et relu identique",
        ] {
            assert!(io.out.contains(s), "missing {s:?} in:\n{}", io.out);
        }
    }

    #[test]
    fn every_outcome_has_its_exit_code() {
        let r = |o: &Outcome, m: Mode| report(o, m, Lang::En, true).1;
        assert_eq!(
            r(
                &Outcome::Verified {
                    backup: None,
                    bluez_name: None
                },
                Mode::Write
            ),
            EXIT_OK
        );
        assert_eq!(r(&Outcome::Cancelled, Mode::Check), EXIT_OK);
        assert_eq!(r(&Outcome::Cancelled, Mode::Write), EXIT_CANCELLED);
        assert_eq!(r(&Outcome::ConfigDisabled, Mode::Write), EXIT_LOCK1);
        assert_eq!(
            r(
                &Outcome::Preflight(vec![PreflightFail::ControlMtuUnknown]),
                Mode::Write
            ),
            EXIT_PREFLIGHT
        );
        assert_eq!(
            r(
                &Outcome::Preflight(vec![PreflightFail::NotConnected]),
                Mode::Check
            ),
            EXIT_PREFLIGHT
        );
        assert_eq!(r(&Outcome::NoReconnect, Mode::Write), EXIT_NO_RECONNECT);
        assert_eq!(
            r(
                &Outcome::Mismatch {
                    read: vec![0; 32],
                    backup: Some("/x".into()),
                    bluez_name: Some("alex".into())
                },
                Mode::Write
            ),
            EXIT_MISMATCH
        );
        for o in [
            Outcome::NotProven,
            Outcome::NoCachedName,
            Outcome::Disconnected,
            Outcome::Refused("r".into()),
            Outcome::WriteFailed("w".into()),
        ] {
            assert_eq!(r(&o, Mode::Write), EXIT_ERROR, "{o:?}");
        }
        let codes = [
            EXIT_OK,
            EXIT_ERROR,
            EXIT_ABSENT,
            EXIT_LOCK1,
            EXIT_PREFLIGHT,
            EXIT_CANCELLED,
            EXIT_NO_RECONNECT,
            EXIT_MISMATCH,
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "distinct exit codes");
        // The French and English verdicts both name the rollback command.
        for l in [Lang::Fr, Lang::En] {
            let t = report(
                &Outcome::Mismatch {
                    read: vec![0; 32],
                    backup: Some("/b.json".into()),
                    bluez_name: None,
                },
                Mode::Write,
                l,
                true,
            )
            .0;
            assert!(t.contains("akmctl rename --device-name --restore /b.json --write-device-name"));
        }
    }

    #[test]
    fn this_command_uses_the_production_proof_and_the_real_locks_only() {
        let src = include_str!("devnamecmd.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains("devname::SEQUENCE_PROOF,"));
        assert!(!prod.contains(&["SequenceProof", "::"].concat()));
        // Lock 1 reaches akm-core as `allow_write`: the configuration, or a
        // terminal where ECRIRE is demanded; --check never writes.
        assert!(
            prod.contains("Consent::Config => true,\n        Consent::Interactive => interactive,")
        );
        assert!(prod.contains(
            "config_allows_write(),\n        lang,\n        &mut world,\n        &mut io,"
        ));
        // The consent is never persisted: this file never writes a file but
        // the backup (through akm-core), never config.toml.
        assert!(
            !prod.contains("fs::write")
                && !prod.contains("OpenOptions")
                && !prod.contains("config::save")
        );
        assert!(prod.contains("config.toml untouched"));
        // The MTU comes from the read-only `inspect` verb, never from a dry run
        // of a HID_CONTROL byte; and the only pkexec call is in hid_control.rs.
        assert!(prod.contains("inspect_control_mtu("));
        assert!(!prod.contains("HidControlOp"));
        assert!(!prod.contains("pkexec\""));
        // No fixed MTU is assumed anywhere in this command.
        assert!(!prod.contains("Some(672)") && !prod.contains("Some(66)"));
        // The consent word is exact, upper case, no accent.
        assert_eq!(CONSENT_WORD, "ECRIRE");
    }
}
