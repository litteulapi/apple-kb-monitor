//! `akmctl rename --device-name ...`: the name stored IN the keyboard (#248).
//!
//! Two names exist (docs/RENOMMER-CLAVIER.md):
//! * `akmctl rename <nom>`: alias of THIS computer (BlueZ `Alias`), the
//!   default, unchanged;
//! * `akmctl rename --device-name <nom>`: the keyboard's own name, stored in
//!   its firmware (`0x51`-`0x54` read, `0x55` written), seen by every host.
//!
//! `akmctl rename --device-name NOM` WRITES the name (measured on the real
//! keyboard on 02/10/2026: the firmware accepts the frame and `0x51`-`0x54`
//! show the new name in the same connection). The flow of
//! [`akm_core::devname::run`]: name validated → pre-flight (connected,
//! battery, breaker, last read complete, `doctor` green, control-channel MTU
//! ≥ 66 read by the single `pkexec akm-hid-control inspect`, no password in
//! the active local session) → backup 0600 of the current name → ONE
//! confirmation (`[o/N]` / `[y/N]`; `--yes` skips it and works without a
//! terminal) → the hidraw door (under the HID lock) → ONE write → immediate
//! read-back of `0x51`-`0x54` through the same door → verdict → the daemon is
//! asked to read the name again (D-Bus `RereadName`).
//!
//! Other modes: `--show` (the daemon's cache, no hardware request),
//! `--dry-run` (the bytes, no pkexec, nothing written), `--check` (the whole
//! pre-flight and the backup, nothing written, nothing asked), `--restore
//! FILE` (writes a backup back, same flow). Without a terminal and without
//! `--yes`: refused before anything is probed.
//!
//! The output is short; `--verbose` (and `--dry-run`) add the byte-by-byte
//! detail and the `[devname]` journal. Messages are in French when
//! `LC_ALL`/`LC_MESSAGES`/`LANG` starts with `fr`, in English otherwise. Exit
//! codes: see [`EXIT_NO_CONFIRM`] and the following.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use akm_core::devname::{
    self, Backup, Frame, NameDoor, Outcome, Preflight, PreflightFail, RenameEnv, Request,
};
use akm_core::parity::FeatureSink;
use akm_core::registry::WriteSession;
use akm_core::Snapshot;

use crate::bus;
use crate::cli::{EXIT_ABSENT, EXIT_ERROR, EXIT_OK};

/// No confirmation possible: not a terminal and no `--yes`. Nothing was
/// touched (no pre-flight, no pkexec, no backup).
pub const EXIT_NO_CONFIRM: u8 = 10;
/// Refused by the pre-flight (control-channel MTU included). Nothing written.
pub const EXIT_PREFLIGHT: u8 = 11;
/// Cancelled: the answer was not a yes. Nothing written (the backup stays).
pub const EXIT_CANCELLED: u8 = 12;
/// Written, but the read-back was not possible: check later with `--show`.
pub const EXIT_UNVERIFIED: u8 = 13;
/// Written, read back DIFFERENT: the exact rollback command was printed.
pub const EXIT_MISMATCH: u8 = 14;

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
    /// The whole pre-flight (one pkexec for the MTU) and the backup; stops
    /// where the confirmation would be asked. Nothing written, nothing asked.
    Check,
    /// The write (the default).
    Write,
}

/// Options of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Opts {
    /// `--yes`: do not ask the confirmation (works without a terminal).
    pub yes: bool,
    /// `--verbose`: frames, wire bytes, proof, `[devname]` journal.
    pub verbose: bool,
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
    fn open_door(&mut self) -> Result<Box<dyn NameDoor>, String>;
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf>;
    fn now_unix(&mut self) -> u64;
    /// Ask the daemon to read the name again (D-Bus `RereadName`).
    fn reread_name(&mut self) -> Result<(), String>;
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
    fn open_door(&mut self) -> Result<Box<dyn NameDoor>, String> {
        akm_core::hidraw::WriteDoor::open().map(|d| Box::new(d) as Box<dyn NameDoor>)
    }
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
        devname::write_backup(&devname::state_dir(), b)
    }
    fn now_unix(&mut self) -> u64 {
        now_unix()
    }
    fn reread_name(&mut self) -> Result<(), String> {
        let conn = bus::connect().map_err(|e| e.to_string())?;
        bus::reread_name(&conn).map_err(|e| e.to_string())
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

/// Pre-flight facts from a snapshot (+ doctor verdict, MTU probe).
pub fn preflight_from(s: &Snapshot, doctor_green: bool, control_mtu: Option<u16>) -> Preflight {
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
        control_mtu,
    }
}

pub fn run(action: Action, mac: Option<String>, opts: Opts) -> u8 {
    let lang = Lang::detect();
    let mut world = RealWorld;
    let mut io = Stdio;
    dispatch(action, mac, opts, lang, &mut world, &mut io)
}

/// The whole command on a given world and terminal (what the tests drive).
pub fn dispatch(
    action: Action,
    mac: Option<String>,
    opts: Opts,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    match action {
        Action::Show => show(lang, world, io),
        Action::Rename { name, mode } => rename(&name, mode, mac, opts, lang, world, io),
        Action::Restore { file, mode } => restore(&file, mode, mac, opts, lang, world, io),
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

/// The detail shown by `--verbose` and `--dry-run`: proof of the frame, what
/// was measured, what is not.
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
    s.push_str(lang.t(
        "Mesuré sur le clavier (micrologiciel 0x0050) :\n",
        "Measured on the keyboard (firmware 0x0050):\n",
    ));
    for u in devname::RESOLVED_BY_MEASUREMENT {
        s.push_str(&format!("  - {u}\n"));
    }
    s.push_str(lang.t("Non mesuré :\n", "Not measured:\n"));
    for u in devname::UNKNOWNS {
        s.push_str(&format!("  - {u}\n"));
    }
    s.push_str(lang.t("Expériences :\n", "Experiments:\n"));
    for e in devname::VALIDATION_EXPERIMENTS {
        s.push_str(&format!("  - {e}\n"));
    }
    s
}

/// The frames, byte for byte, with their proof (`--verbose`, `--dry-run`).
fn frames_text(frames: &[Frame], lang: Lang) -> String {
    format!(
        "{}\n{}",
        lang.t(
            "Trame envoyée (celle de Lion 10.7.5 setDeviceName:, établie par désassemblage) :",
            "Frame sent (the one of Lion 10.7.5 setDeviceName:, established by disassembly):"
        ),
        devname::render_frames(frames)
    )
}

fn fail_text(f: &PreflightFail, lang: Lang) -> String {
    if lang == Lang::En {
        return f.describe();
    }
    match f {
        PreflightFail::NotConnected => "clavier non connecté".into(),
        PreflightFail::Battery => {
            "batterie sous 20 % et état différent de « normal » (ou inconnu)".into()
        }
        PreflightFail::BreakerOpen => "disjoncteur ouvert : le clavier a cessé de répondre".into(),
        PreflightFail::RecentReadFailure => {
            "la dernière lecture matérielle a échoué ou est incomplète".into()
        }
        PreflightFail::DoctorNotGreen => {
            "`akmctl doctor` n'est pas vert (liaison, appairage, configuration)".into()
        }
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

/// The pre-flight in one line when it passes (`with_mtu` false: dry run, the
/// MTU is not probed); the failures are listed by [`report`].
fn preflight_ok_line(p: &Preflight, lang: Lang, with_mtu: bool) -> String {
    format!(
        "{} ({}{}{}{})\n",
        lang.t("Pré-vol : ok", "Pre-flight: ok"),
        lang.t("doctor vert, ", "doctor green, "),
        match p.battery_pct {
            Some(b) => format!("{} {b:.0} %, ", lang.t("batterie", "battery")),
            None => String::new(),
        },
        lang.t("disjoncteur fermé", "breaker closed"),
        match p.control_mtu {
            Some(m) if with_mtu => format!(
                ", {} {m} >= {}",
                lang.t("MTU sortante", "outgoing MTU"),
                devname::MIN_CONTROL_MTU
            ),
            _ => String::new(),
        }
    )
}

/// The exact rollback command for a backup.
pub fn restore_command(backup: &Path) -> String {
    format!(
        "akmctl rename --device-name --restore {} --yes",
        backup.display()
    )
}

/// Not a terminal and no `--yes`: refused before anything is probed.
fn refuse_no_confirmation(lang: Lang, io: &mut dyn Io) -> u8 {
    io.out(lang.t(
        "Refusé : pas de terminal pour confirmer. Ajoutez --yes pour écrire sans question. Rien n'a été touché.\n",
        "Refused: no terminal to confirm. Add --yes to write without the question. Nothing was touched.\n",
    ));
    EXIT_NO_CONFIRM
}

fn rename(
    name: &str,
    mode: Mode,
    mac: Option<String>,
    opts: Opts,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
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
    match mode {
        Mode::DryRun => dry_run(name, &frames, mac, lang, world, io),
        Mode::Check | Mode::Write => guarded(
            &Request::Rename(name.to_string()),
            frames,
            mode,
            mac,
            opts,
            lang,
            world,
            io,
        ),
    }
}

fn dry_run(
    name: &str,
    frames: &[Frame],
    mac: Option<String>,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    io.out(lang.t(TWO_NAMES_FR, TWO_NAMES));
    io.out(&format!(
        "\n{} {name:?}\n{}",
        lang.t(
            "Nouveau nom stocké dans le clavier :",
            "New name stored in the keyboard:"
        ),
        frames_text(frames, lang)
    ));
    io.out(lang.t(
        "\nESSAI À BLANC : rien n'est écrit dans le clavier.\n",
        "\nDRY RUN: nothing is written to the keyboard.\n",
    ));
    match world.snapshot() {
        Ok(s) => {
            let green = world.doctor_green(mac.as_deref());
            let p = preflight_from(&s, green, None);
            let fails: Vec<_> = devname::preflight(&p)
                .into_iter()
                .filter(|f| !f.is_mtu())
                .collect();
            if fails.is_empty() {
                io.out(&preflight_ok_line(&p, lang, false));
            }
            for f in &fails {
                io.out(&format!(
                    "{} {}\n",
                    lang.t("Pré-vol : ÉCHEC -", "Pre-flight: FAILED -"),
                    fail_text(f, lang)
                ));
            }
            io.out(&format!(
                "{} {} {}\n",
                lang.t(
                    "Pré-vol : la MTU du canal de contrôle n'est pas lue en essai à blanc (elle demande pkexec) ; lue et exigée >=",
                    "Pre-flight: control-channel MTU not probed in a dry run (needs pkexec); probed and required >="
                ),
                devname::MIN_CONTROL_MTU,
                lang.t("avec --check ou à l'écriture", "with --check or when writing")
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
                        Err(e) => io.out(&format!(
                            "{} {e}\n",
                            lang.t("Sauvegarde : ÉCHEC -", "Backup: FAILED -")
                        )),
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
    io.out(lang.t(
        "\nPour écrire : la même commande sans --dry-run (une confirmation [o/N], ou --yes). Pour tout vérifier sans écrire : --check.\n",
        "\nTo write: the same command without --dry-run (one confirmation [y/N], or --yes). To check everything without writing: --check.\n",
    ));
    EXIT_OK
}

fn restore(
    file: &Path,
    mode: Mode,
    mac: Option<String>,
    opts: Opts,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    let refused = |io: &mut dyn Io, e: &str| {
        io.journal(&format!(
            "akmctl: {} {e}",
            lang.t("sauvegarde refusée :", "backup refused:")
        ));
        EXIT_ERROR
    };
    let b = match devname::read_backup(file) {
        Ok(b) => b,
        Err(e) => return refused(io, &e),
    };
    let frames = match devname::frames_for_restore(&b) {
        Ok(f) => f,
        Err(e) => return refused(io, &e),
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
    if mode == Mode::DryRun {
        io.out(&frames_text(&frames, lang));
        io.out(lang.t(
            "ESSAI À BLANC : rien d'écrit. Sans --dry-run, la sauvegarde est réécrite dans le clavier (une confirmation [o/N], ou --yes).\n",
            "DRY RUN: nothing written. Without --dry-run, the backup is written back into the keyboard (one confirmation [y/N], or --yes).\n",
        ));
        return EXIT_OK;
    }
    guarded(
        &Request::Restore(b),
        frames,
        mode,
        mac,
        opts,
        lang,
        world,
        io,
    )
}

// ── the guarded flow ───────────────────────────────────────────────────────

/// The real environment of [`devname::run`]: daemon snapshot, doctor, MTU
/// probe (one `pkexec` per command), backup dir, the single confirmation,
/// the hidraw door (write, then read-back).
struct Flow<'a> {
    world: &'a mut dyn World,
    io: &'a mut dyn Io,
    lang: Lang,
    mode: Mode,
    opts: Opts,
    mac: Option<String>,
    /// The name being written (shown in the header and the question).
    target: String,
    door: Option<Box<dyn NameDoor>>,
    /// Result of the single MTU probe of this command (`None` = not run yet).
    mtu: Option<Result<u16, String>>,
    probes: u32,
    preflights: u32,
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

/// Is this answer a yes? (`o`, `oui` in French; `y`, `yes` in both.)
fn is_yes(answer: &str, lang: Lang) -> bool {
    let a = answer.trim().to_lowercase();
    matches!(a.as_str(), "y" | "yes") || (lang == Lang::Fr && matches!(a.as_str(), "o" | "oui"))
}

impl Flow<'_> {
    fn verbose(&mut self, line: &str) {
        if self.opts.verbose {
            self.io.journal(line);
        }
    }

    fn snapshot(&mut self) -> Option<Snapshot> {
        match self.world.snapshot() {
            Ok(s) => Some(s),
            Err((_, m)) => {
                self.io.journal(&format!("akmctl: daemon unavailable: {m}"));
                None
            }
        }
    }

    /// The outgoing MTU of the control channel, probed once (the second
    /// pre-flight call, just before the write, reuses the value).
    fn control_mtu(&mut self, snapshot_mac: Option<&str>) -> Option<u16> {
        if self.mtu.is_none() {
            let Some(mac) = self
                .mac
                .clone()
                .or_else(|| snapshot_mac.map(str::to_string))
            else {
                self.verbose("[devname] MTU probe skipped: the keyboard's MAC is unknown");
                self.mtu = Some(Err("MAC unknown".into()));
                return None;
            };
            self.verbose(&format!(
                "[devname] reading the outgoing MTU of the L2CAP control channel to {mac} (pkexec akm-hid-control inspect --mac {mac}: read-only, nothing sent)"
            ));
            self.probes += 1;
            let r = match self.world.probe_control_mtu(&mac) {
                Ok((m, text)) => {
                    for l in text
                        .lines()
                        .filter(|l| l.contains(" fd ") || l.contains("inspected"))
                    {
                        self.verbose(&format!("[devname]   {}", l.trim()));
                    }
                    self.verbose(&format!(
                        "[devname] control-channel outgoing MTU = {m} (required >= {})",
                        devname::MIN_CONTROL_MTU
                    ));
                    Ok(m)
                }
                Err(e) => {
                    // Always said: it explains the pre-flight refusal that follows.
                    self.io
                        .journal(&format!("akmctl: control-channel MTU unknown: {e}"));
                    Err(e)
                }
            };
            self.mtu = Some(r);
        }
        self.mtu.as_ref().and_then(|r| r.as_ref().ok().copied())
    }
}

impl RenameEnv for Flow<'_> {
    fn preflight(&mut self) -> Preflight {
        self.preflights += 1;
        let Some(s) = self.snapshot() else {
            return Preflight::default();
        };
        if self.preflights == 1 {
            let current = s
                .keyboard
                .as_ref()
                .and_then(|k| k.device.name_on_keyboard.clone());
            self.io.out(&format!(
                "{} {} → « {} »\n",
                self.lang.t(
                    "Nom stocké dans le clavier :",
                    "Name stored in the keyboard:"
                ),
                match current {
                    Some(n) => format!("« {n} »"),
                    None => self.lang.t("(pas encore lu)", "(not read yet)").to_string(),
                },
                self.target
            ));
        }
        let mtu = self.control_mtu(snapshot_mac(&s).as_deref());
        let mac = self.mac.clone();
        let green = self.world.doctor_green(mac.as_deref());
        let p = preflight_from(&s, green, mtu);
        if self.preflights == 1 && devname::preflight(&p).is_empty() {
            let line = preflight_ok_line(&p, self.lang, true);
            self.io.out(&line);
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
        self.io.out(&format!(
            "{} {}\n",
            self.lang
                .t("Sauvegarde du nom actuel :", "Backup of the current name:"),
            p.display()
        ));
        Ok(p)
    }
    fn confirm(&mut self, target: &str) -> bool {
        let l = self.lang;
        if self.mode == Mode::Check {
            return false;
        }
        if !self.opts.yes {
            self.io.out(&format!(
                "{} « {target} » {} ",
                l.t("Écrire", "Write"),
                l.t(
                    "dans la mémoire du clavier ? [o/N]",
                    "into the keyboard's memory? [y/N]"
                )
            ));
            let answer = self.io.read_line().unwrap_or_default();
            if !is_yes(&answer, l) {
                return false;
            }
        }
        // Only now is the hardware node opened (under the HID lock).
        match self.world.open_door() {
            Ok(d) => self.door = Some(d),
            Err(e) => {
                self.verbose(&format!("[devname] write door not opened: {e}"));
                self.door_error = Some(e);
            }
        }
        true
    }
    fn sink(&self) -> &dyn FeatureSink {
        match &self.door {
            Some(d) => d.as_sink(),
            None => &CLOSED,
        }
    }
    fn read_back(&mut self) -> std::io::Result<Vec<u8>> {
        // The door is dropped right after: the HID lock goes back to the daemon.
        match self.door.take() {
            Some(d) => d.read_name(),
            None => Err(std::io::Error::other("no hidraw door")),
        }
    }
    fn log(&mut self, line: &str) {
        self.verbose(line);
    }
}

#[allow(clippy::too_many_arguments)]
fn guarded(
    req: &Request,
    frames: Vec<Frame>,
    mode: Mode,
    mac: Option<String>,
    opts: Opts,
    lang: Lang,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    // No way to confirm: refused before any pre-flight, pkexec or backup.
    if mode == Mode::Write && !opts.yes && !io.interactive() {
        return refuse_no_confirmation(lang, io);
    }
    let target = match req {
        Request::Rename(n) => n.clone(),
        Request::Restore(b) => b.name.clone(),
    };
    if opts.verbose {
        io.out(&frames_text(&frames, lang));
        io.out(&proof_state(lang));
    }
    let mut flow = Flow {
        world,
        io,
        lang,
        mode,
        opts,
        mac,
        target: target.clone(),
        door: None,
        mtu: None,
        probes: 0,
        preflights: 0,
        door_error: None,
    };
    let mut session = WriteSession::new();
    let o = devname::run(req, devname::SEQUENCE_PROOF, &mut session, &mut flow);
    debug_assert!(flow.probes <= 1, "one pkexec per command");
    flow.door = None;
    if let (Outcome::WriteFailed(_), Some(d)) = (&o, &flow.door_error) {
        // The door never opened: the closed sink refused, nothing left akmctl.
        flow.io.out(lang.t(
            &format!("Arrêt : le nœud hidraw ne s'est pas ouvert ({d}). Rien n'a été écrit.\n"),
            &format!("Stopped: the hidraw node did not open ({d}). Nothing was written.\n"),
        ));
        return EXIT_ERROR;
    }
    if matches!(
        o,
        Outcome::Verified { .. } | Outcome::Mismatch { .. } | Outcome::Unverified { .. }
    ) {
        // The daemon's cached name is stale: ask it to read 0x51-0x54 again.
        if let Err(e) = flow.world.reread_name() {
            flow.verbose(&format!("[devname] RereadName not delivered: {e}"));
        }
    }
    let (text, code) = report(&o, mode, lang, &target);
    flow.io.out(&text);
    code
}

/// What stays true after a write, in one line.
fn after_write_note(lang: Lang) -> &'static str {
    lang.t(
        "  BlueZ peut afficher l'ancien nom jusqu'à une prochaine connexion ; la persistance après un changement de piles n'est pas mesurée.\n",
        "  BlueZ may show the old name until a later connection; persistence across a battery change is not measured.\n",
    )
}

/// Text for the user and exit code of an outcome (`target` = the name asked).
pub fn report(o: &Outcome, mode: Mode, lang: Lang, target: &str) -> (String, u8) {
    let l = lang;
    match o {
        Outcome::NotProven => (
            l.t(
                "Refusé (NotProven) : les octets exacts qu'Apple envoie ne sont pas établis ; rien n'a été touché.\n",
                "Refused (NotProven): the exact bytes Apple sends are not established; nothing was touched.\n",
            )
            .to_string(),
            EXIT_ERROR,
        ),
        Outcome::Preflight(fails) => {
            let mut s = String::from(l.t(
                "Pré-vol refusé, rien n'a été écrit :\n",
                "Pre-flight refused, nothing was written:\n",
            ));
            for f in fails {
                s.push_str(&format!("  - {}\n", fail_text(f, l)));
            }
            (s, EXIT_PREFLIGHT)
        }
        Outcome::Cancelled if mode == Mode::Check => (
            l.t(
                "✓ --check : pré-vol vert, sauvegarde faite, rien n'a été écrit, rien n'a été demandé.\n",
                "✓ --check: pre-flight green, backup made, nothing was written, nothing was asked.\n",
            )
            .to_string(),
            EXIT_OK,
        ),
        Outcome::Cancelled => (
            l.t(
                "Annulé : rien n'a été écrit dans le clavier.\n",
                "Cancelled: nothing was written into the keyboard.\n",
            )
            .to_string(),
            EXIT_CANCELLED,
        ),
        Outcome::Verified { .. } => (
            format!(
                "✓ {} « {target} »\n{}",
                l.t(
                    "Nom écrit dans le clavier et relu identique :",
                    "Name written into the keyboard and read back identical:"
                ),
                after_write_note(l)
            ),
            EXIT_OK,
        ),
        Outcome::Mismatch { read, backup } => {
            let mut s = format!(
                "✗ {} {} ({})\n",
                l.t(
                    "Écrit, mais le nom relu diffère :",
                    "Written, but the name read back differs:"
                ),
                match devname::name_from_raw(read) {
                    Some(n) => format!("« {n} »"),
                    None => l.t("(non imprimable)", "(not printable)").to_string(),
                },
                devname::hex(read)
            );
            match backup {
                Some(b) => s.push_str(&format!(
                    "  {} {}\n",
                    l.t("Retour arrière :", "Rollback:"),
                    restore_command(b)
                )),
                None => s.push_str(l.t(
                    "  Retour arrière : relancer --restore avec la sauvegarde d'origine.\n",
                    "  Rollback: run --restore again with the original backup.\n",
                )),
            }
            (s, EXIT_MISMATCH)
        }
        Outcome::Unverified { backup, error } => {
            let mut s = format!(
                "✎ {} ({error}).\n  {} akmctl rename --device-name --show\n",
                l.t(
                    "Nom écrit, mais la relecture n'a pas été possible",
                    "Name written, but the read-back was not possible"
                ),
                l.t("Vérifiez plus tard :", "Check later:")
            );
            if let Some(b) = backup {
                s.push_str(&format!(
                    "  {} {}\n",
                    l.t("Retour arrière si besoin :", "Rollback if needed:"),
                    restore_command(b)
                ));
            }
            s.push_str(after_write_note(l));
            (s, EXIT_UNVERIFIED)
        }
        Outcome::NoCachedName => (
            l.t(
                "Arrêt : le démon n'a pas encore lu le nom actuel (0x51-0x54) dans cette connexion, la sauvegarde est impossible. Réessayez dans un instant. Rien n'a été écrit.\n",
                "Stopped: the daemon has not read the current name (0x51-0x54) in this connection yet, no backup is possible. Try again in a moment. Nothing was written.\n",
            )
            .to_string(),
            EXIT_ERROR,
        ),
        other => (
            format!(
                "{} {other:?}. {}\n",
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

    const MAC: &str = "AA:BB:CC:DD:EE:F1";
    /// "Clavier de alice #1", the 4 × 8 bytes of 0x51-0x54 as the daemon caches them.
    const OLD_HEX: &str = concat!(
        "436c617669657220",
        "646520616c696365",
        "2023310000000000",
        "0000000000000000"
    );
    const BACKUP: &str = "/sim/devname-backup-20260930T235959Z.json";

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
        k.device.name = Some("Clavier de alice #1".into());
        k.device.name_on_keyboard_hex = raw_hex.map(str::to_string);
        k.device.name_on_keyboard = raw_hex
            .and_then(devname::unhex)
            .and_then(|r| devname::name_from_raw(&r));
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
    type Events = Rc<RefCell<Vec<String>>>;

    /// What the fake door reads back from 0x51-0x54.
    #[derive(Clone)]
    enum Back {
        /// What was written (the first 32 data bytes of the frame).
        Echo,
        Hex(String),
        Error(String),
    }

    /// The fake door: a spy sink and a scripted read-back. No hardware.
    struct FakeDoor {
        writes: Writes,
        events: Events,
        back: Back,
    }

    impl FeatureSink for FakeDoor {
        fn set_feature(&self, op: WriteOp, report: &[u8]) -> std::io::Result<()> {
            self.writes.borrow_mut().push((op, report.to_vec()));
            self.events
                .borrow_mut()
                .push(format!("write {:#04x}", report[0]));
            Ok(())
        }
    }

    impl NameDoor for FakeDoor {
        fn read_name(&self) -> std::io::Result<Vec<u8>> {
            self.events.borrow_mut().push("read back".into());
            match &self.back {
                Back::Echo => Ok(self.writes.borrow()[0].1[1..33].to_vec()),
                Back::Hex(h) => Ok(devname::unhex(h).unwrap()),
                Back::Error(e) => Err(std::io::Error::other(e.clone())),
            }
        }
        fn as_sink(&self) -> &dyn FeatureSink {
            self
        }
    }

    struct FakeWorld {
        connected: bool,
        raw_hex: Option<String>,
        doctor: bool,
        mtu: Result<(u16, String), String>,
        probes: u32,
        door_fails: bool,
        back: Back,
        writes: Writes,
        events: Events,
        backups: Vec<Backup>,
    }

    impl FakeWorld {
        fn green() -> Self {
            Self {
                connected: true,
                raw_hex: Some(OLD_HEX.into()),
                doctor: true,
                mtu: Ok((185, format!("akm-hid-control:   {MAC} fd 23: psm local 0x0000 peer 0x0011 cid 0x0041 state connected (hci handle 0x000b) mtu out 185 in 672\nakm-hid-control: {MAC}: inspected, nothing sent"))),
                probes: 0,
                door_fails: false,
                back: Back::Echo,
                writes: Rc::default(),
                events: Rc::default(),
                backups: Vec::new(),
            }
        }
    }

    impl World for FakeWorld {
        fn snapshot(&mut self) -> Result<Snapshot, (u8, String)> {
            self.events.borrow_mut().push("snapshot".into());
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
        fn open_door(&mut self) -> Result<Box<dyn NameDoor>, String> {
            self.events.borrow_mut().push("door".into());
            if self.door_fails {
                return Err("no Apple keyboard found (hidraw)".into());
            }
            Ok(Box::new(FakeDoor {
                writes: self.writes.clone(),
                events: self.events.clone(),
                back: self.back.clone(),
            }))
        }
        fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
            self.events.borrow_mut().push("backup".into());
            self.backups.push(b.clone());
            Ok(PathBuf::from(BACKUP))
        }
        fn now_unix(&mut self) -> u64 {
            1_790_812_799
        }
        fn reread_name(&mut self) -> Result<(), String> {
            self.events.borrow_mut().push("RereadName".into());
            Ok(())
        }
    }

    /// The events without the D-Bus `GetState` calls (which probe nothing).
    fn acts(w: &FakeWorld) -> Vec<String> {
        w.events
            .borrow()
            .iter()
            .filter(|e| *e != "snapshot")
            .cloned()
            .collect()
    }

    const YES: Opts = Opts {
        yes: true,
        verbose: false,
    };
    const ASK: Opts = Opts {
        yes: false,
        verbose: false,
    };

    fn write(name: &str, opts: Opts, w: &mut FakeWorld, io: &mut FakeIo) -> u8 {
        dispatch(
            Action::Rename {
                name: name.into(),
                mode: Mode::Write,
            },
            None,
            opts,
            Lang::En,
            w,
            io,
        )
    }

    #[test]
    fn preflight_from_a_snapshot() {
        let s = snap(true, Some(OLD_HEX));
        let p = preflight_from(&s, true, Some(672));
        assert!(devname::preflight(&p).is_empty(), "{p:?}");
        assert_eq!(
            devname::name_from_raw(&cached_raw(&s).unwrap()).unwrap(),
            "Clavier de alice #1"
        );
        // MTU unknown or too small is a pre-flight failure of its own.
        let p = preflight_from(&s, true, None);
        assert_eq!(
            devname::preflight(&p),
            vec![PreflightFail::ControlMtuUnknown]
        );
        let p = preflight_from(&s, true, Some(48));
        assert_eq!(
            devname::preflight(&p),
            vec![PreflightFail::ControlMtuTooSmall(48)]
        );
        let mut s2 = s.clone();
        s2.keyboard.as_mut().unwrap().breaker_open = true;
        s2.keyboard.as_mut().unwrap().incomplete = true;
        let f = devname::preflight(&preflight_from(&s2, false, Some(672)));
        assert_eq!(f.len(), 3);
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
    fn no_tty_and_no_yes_refuses_before_any_probe() {
        for restore in [false, true] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(false, &["y", "y"]);
            let code = if restore {
                let dir =
                    std::env::temp_dir().join(format!("akm-devnamecmd-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                let b =
                    Backup::new(MAC, &devname::unhex(OLD_HEX).unwrap(), 1, "daemon-cache").unwrap();
                let file = devname::write_backup(&dir, &b).unwrap();
                let c = dispatch(
                    Action::Restore {
                        file,
                        mode: Mode::Write,
                    },
                    None,
                    ASK,
                    Lang::En,
                    &mut w,
                    &mut io,
                );
                std::fs::remove_dir_all(&dir).unwrap();
                c
            } else {
                write("Bureau", ASK, &mut w, &mut io)
            };
            assert_eq!(code, EXIT_NO_CONFIRM);
            assert_eq!(w.probes, 0, "no pkexec");
            assert!(
                w.events.borrow().is_empty(),
                "no daemon call, no pre-flight, no backup, no door: {:?}",
                w.events.borrow()
            );
            assert!(w.writes.borrow().is_empty());
            assert!(io.out.contains("--yes") && io.out.contains("Nothing was touched"));
            // Lines piped in are never read as an answer.
            assert_eq!(io.lines.len(), 2);
        }
    }

    #[test]
    fn yes_without_a_tty_sends_exactly_the_fixture_frame_once_after_the_backup() {
        let fx = fixture();
        for ex in fx["examples"].as_array().unwrap() {
            let name = ex["name"].as_str().unwrap();
            let want = devname::unhex(ex["report_hex"].as_str().unwrap()).unwrap();
            let mut w = FakeWorld::green();
            w.back = Back::Hex(
                ex["readback_0x51_0x54_expected_hex"]
                    .as_str()
                    .unwrap()
                    .into(),
            );
            let mut io = FakeIo::new(false, &["never read"]);
            assert_eq!(write(name, YES, &mut w, &mut io), EXIT_OK, "{name}");
            assert_eq!(
                acts(&w),
                vec![
                    "pkexec",
                    "backup",
                    "door",
                    "write 0x55",
                    "read back",
                    "RereadName"
                ],
                "{name}: one probe, the backup, then the single write and its read-back"
            );
            assert_eq!(w.probes, 1, "{name}: exactly one pkexec");
            let wr = w.writes.borrow();
            assert_eq!(wr.len(), 1);
            assert_eq!(wr[0].0, WriteOp::DeviceName);
            assert_eq!(wr[0].1, want, "{name}: the fixture bytes, exactly");
            assert_eq!(wr[0].1.len(), 65);
            assert_eq!(w.backups[0].name, "Clavier de alice #1");
            assert_eq!(io.lines.len(), 1, "{name}: nothing is asked with --yes");
            assert!(!io.out.contains("[y/N]"), "{name}");
            assert!(io.out.contains("read back identical"), "{name}");
        }
    }

    #[test]
    fn one_question_and_a_yes_writes_a_no_writes_nothing() {
        // Yes.
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["y", "left"]);
        assert_eq!(write("Bureau", ASK, &mut w, &mut io), EXIT_OK);
        assert_eq!(io.out.matches("[y/N]").count(), 1, "ONE question");
        assert_eq!(io.lines.len(), 1, "one line read");
        assert_eq!(w.writes.borrow().len(), 1);
        // The short output, in order, and nothing of the detail.
        let want = format!(
            "Name stored in the keyboard: « Clavier de alice #1 » → « Bureau »\n\
             Pre-flight: ok (doctor green, battery 99 %, breaker closed, outgoing MTU 185 >= 66)\n\
             Backup of the current name: {BACKUP}\n\
             Write « Bureau » into the keyboard's memory? [y/N] \
             ✓ Name written into the keyboard and read back identical: « Bureau »\n  \
             BlueZ may show the old name until a later connection; persistence across a battery change is not measured.\n"
        );
        assert_eq!(io.out, want);
        assert!(
            io.journal.is_empty(),
            "no journal without --verbose: {:?}",
            io.journal
        );
        // No, empty, anything else, end of input: nothing written, code 12.
        for answer in [
            &["n"][..],
            &[""][..],
            &["N"][..],
            &["o"][..],
            &["yess"][..],
            &[][..],
        ] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(true, answer);
            assert_eq!(
                write("Bureau", ASK, &mut w, &mut io),
                EXIT_CANCELLED,
                "{answer:?}"
            );
            assert_eq!(
                acts(&w),
                vec!["pkexec", "backup"],
                "{answer:?}: no door, no write"
            );
            assert!(w.writes.borrow().is_empty());
            assert!(io.out.contains("Cancelled: nothing was written"));
        }
        // French: o / oui / O are a yes; the question is [o/N].
        for answer in ["o", "oui", "O", "y"] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(true, &[answer]);
            let code = dispatch(
                Action::Rename {
                    name: "Bureau".into(),
                    mode: Mode::Write,
                },
                Some(MAC.into()),
                ASK,
                Lang::Fr,
                &mut w,
                &mut io,
            );
            assert_eq!(code, EXIT_OK, "{answer}");
            assert!(io
                .out
                .contains("Écrire « Bureau » dans la mémoire du clavier ? [o/N]"));
            assert!(io
                .out
                .contains("✓ Nom écrit dans le clavier et relu identique : « Bureau »"));
            assert!(
                io.out.contains("Pré-vol : ok") && io.out.contains("Sauvegarde du nom actuel :")
            );
            assert!(io.out.contains("changement de piles"));
        }
        assert!(is_yes(" Yes \n", Lang::En) && !is_yes("o", Lang::En) && !is_yes("non", Lang::Fr));
    }

    #[test]
    fn read_back_different_is_14_with_the_exact_rollback_command() {
        let mut w = FakeWorld::green();
        w.back = Back::Hex(OLD_HEX.into());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Bureau", YES, &mut w, &mut io), EXIT_MISMATCH);
        assert_eq!(w.writes.borrow().len(), 1, "no retry");
        assert_eq!(
            acts(&w).iter().filter(|e| *e == "read back").count(),
            1,
            "one read-back, no retry"
        );
        assert!(io
            .out
            .contains("the name read back differs: « Clavier de alice #1 »"));
        assert!(io.out.contains(&format!(
            "akmctl rename --device-name --restore {BACKUP} --yes"
        )));
        assert_eq!(
            restore_command(Path::new(BACKUP)),
            format!("akmctl rename --device-name --restore {BACKUP} --yes")
        );
    }

    #[test]
    fn read_back_in_error_is_13_and_says_to_check_later() {
        let mut w = FakeWorld::green();
        w.back = Back::Error("report 0x52: no answer".into());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Bureau", YES, &mut w, &mut io), EXIT_UNVERIFIED);
        assert_eq!(w.writes.borrow().len(), 1, "written once, never rewritten");
        assert!(io
            .out
            .contains("the read-back was not possible (report 0x52: no answer)"));
        assert!(io.out.contains("akmctl rename --device-name --show"));
        assert!(acts(&w).contains(&"RereadName".to_string()));
    }

    #[test]
    fn check_runs_the_whole_pre_flight_with_one_pkexec_and_never_asks_nor_writes() {
        let check = |w: &mut FakeWorld, io: &mut FakeIo, opts: Opts| {
            dispatch(
                Action::Rename {
                    name: "Bureau".into(),
                    mode: Mode::Check,
                },
                None,
                opts,
                Lang::En,
                w,
                io,
            )
        };
        for (tty, opts) in [(true, ASK), (false, ASK), (false, YES), (true, YES)] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(tty, &["y", "y"]);
            assert_eq!(check(&mut w, &mut io, opts), EXIT_OK, "tty {tty}");
            assert_eq!(w.probes, 1, "tty {tty}: exactly one pkexec");
            assert_eq!(
                acts(&w),
                vec!["pkexec", "backup"],
                "no door, no write, no RereadName"
            );
            assert!(w.writes.borrow().is_empty());
            assert_eq!(io.lines.len(), 2, "nothing is read from the terminal");
            assert!(!io.out.contains("[y/N]"), "nothing is asked");
            assert!(io.out.contains("✓ --check") && io.out.contains("outgoing MTU 185 >= 66"));
        }
        // MTU too small: refused by the pre-flight, no backup.
        let mut w = FakeWorld::green();
        w.mtu = Ok((48, String::new()));
        let mut io = FakeIo::new(true, &[]);
        assert_eq!(check(&mut w, &mut io, ASK), EXIT_PREFLIGHT);
        assert_eq!(acts(&w), vec!["pkexec"]);
        assert!(io.out.contains("Pre-flight refused") && io.out.contains("48"));
        // pkexec refused: unknown MTU, refused, and the reason is said.
        let mut w = FakeWorld::green();
        w.mtu = Err("authentication dismissed or not authorized (pkexec 126)".into());
        let mut io = FakeIo::new(true, &[]);
        assert_eq!(check(&mut w, &mut io, ASK), EXIT_PREFLIGHT);
        assert!(io.journal.iter().any(|l| l.contains("pkexec 126")));
    }

    #[test]
    fn pre_flight_refusals_list_every_reason_and_write_nothing() {
        let mut w = FakeWorld::green();
        w.connected = false;
        w.doctor = false;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Bureau", YES, &mut w, &mut io), EXIT_PREFLIGHT);
        assert_eq!(acts(&w), vec!["pkexec"], "no backup, no door, no write");
        assert!(io.out.contains("keyboard not connected") && io.out.contains("doctor"));
        // The daemon has no name yet: no backup possible, nothing written.
        let mut w = FakeWorld::green();
        w.raw_hex = None;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Bureau", YES, &mut w, &mut io), EXIT_ERROR);
        assert!(w.writes.borrow().is_empty() && io.out.contains("Nothing was written"));
        // A door that cannot open: nothing written, distinct stop.
        let mut w = FakeWorld::green();
        w.door_fails = true;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Bureau", YES, &mut w, &mut io), EXIT_ERROR);
        assert!(w.writes.borrow().is_empty());
        assert!(io.out.contains("Nothing was written"));
        assert!(!acts(&w).contains(&"RereadName".to_string()));
    }

    #[test]
    fn dry_run_never_probes_asks_nor_writes_and_shows_the_bytes() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["y"]);
        let code = dispatch(
            Action::Rename {
                name: "Bureau".into(),
                mode: Mode::DryRun,
            },
            None,
            ASK,
            Lang::Fr,
            &mut w,
            &mut io,
        );
        assert_eq!(code, EXIT_OK);
        assert_eq!(w.probes, 0);
        assert_eq!(acts(&w), vec!["backup"]);
        assert_eq!(io.lines.len(), 1);
        for s in [
            "ESSAI À BLANC",
            "55 42 75 72 65 61 75",
            "wire   : 53 55",
            "U4 [mesuré le 02/10/2026]",
            "U5 [mesuré le 02/10/2026]",
            "U3 NOT MEASURED",
            "Deux noms existent",
        ] {
            assert!(io.out.contains(s), "missing {s:?} in:\n{}", io.out);
        }
    }

    #[test]
    fn verbose_adds_the_bytes_and_the_journal() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(false, &[]);
        let opts = Opts {
            yes: true,
            verbose: true,
        };
        assert_eq!(write("Bureau", opts, &mut w, &mut io), EXIT_OK);
        assert!(io.out.contains("wire   : 53 55 42 75 72 65 61 75"));
        assert!(io.out.contains("Proof of the sequence"));
        for s in [
            "[devname] pre-flight ok",
            "[devname] control-channel outgoing MTU = 185",
            "[devname] backup written",
            "[devname] write DeviceName Feature 0x55, 65 bytes",
            "[devname] read back 0x51-0x54: 42 75 72 65 61 75 00",
            "[devname] verified",
        ] {
            assert!(
                io.journal.iter().any(|l| l.contains(s)),
                "missing {s:?}: {:?}",
                io.journal
            );
        }
        // Without --verbose: none of it.
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Bureau", YES, &mut w, &mut io), EXIT_OK);
        assert!(!io.out.contains("wire") && !io.out.contains("Proof"));
        assert!(io.journal.is_empty());
    }

    #[test]
    fn restore_writes_the_backup_back_with_the_same_flow() {
        let dir = std::env::temp_dir().join(format!("akm-devnamecmd-r-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = devname::unhex(OLD_HEX).unwrap();
        let b = Backup::new(MAC, &old, 1, "daemon-cache").unwrap();
        let file = devname::write_backup(&dir, &b).unwrap();
        let restore = |w: &mut FakeWorld, io: &mut FakeIo, opts: Opts| {
            dispatch(
                Action::Restore {
                    file: file.clone(),
                    mode: Mode::Write,
                },
                None,
                opts,
                Lang::En,
                w,
                io,
            )
        };
        let bureau = devname::hex(&devname::expected_readback(
            &devname::prepare("Bureau").unwrap(),
        ))
        .replace(' ', "");
        // --yes, no terminal: written once, no new backup.
        let mut w = FakeWorld::green();
        w.raw_hex = Some(bureau.clone());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(restore(&mut w, &mut io, YES), EXIT_OK);
        assert_eq!(
            acts(&w),
            vec!["pkexec", "door", "write 0x55", "read back", "RereadName"]
        );
        assert_eq!(&w.writes.borrow()[0].1[1..33], &old[..]);
        assert!(io
            .out
            .contains("read back identical: « Clavier de alice #1 »"));
        // In a terminal: one question; "n" writes nothing.
        let mut w = FakeWorld::green();
        w.raw_hex = Some(bureau);
        let mut io = FakeIo::new(true, &["n"]);
        assert_eq!(restore(&mut w, &mut io, ASK), EXIT_CANCELLED);
        assert_eq!(io.out.matches("[y/N]").count(), 1);
        assert!(w.writes.borrow().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_outcome_has_its_exit_code() {
        let r = |o: &Outcome, m: Mode| report(o, m, Lang::En, "x").1;
        assert_eq!(r(&Outcome::Verified { backup: None }, Mode::Write), EXIT_OK);
        assert_eq!(r(&Outcome::Cancelled, Mode::Check), EXIT_OK);
        assert_eq!(r(&Outcome::Cancelled, Mode::Write), EXIT_CANCELLED);
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
        assert_eq!(
            r(
                &Outcome::Unverified {
                    backup: None,
                    error: "e".into()
                },
                Mode::Write
            ),
            EXIT_UNVERIFIED
        );
        assert_eq!(
            r(
                &Outcome::Mismatch {
                    read: vec![0; 32],
                    backup: Some("/x".into()),
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
        assert_eq!(
            [
                EXIT_NO_CONFIRM,
                EXIT_PREFLIGHT,
                EXIT_CANCELLED,
                EXIT_UNVERIFIED,
                EXIT_MISMATCH
            ],
            [10, 11, 12, 13, 14]
        );
        assert!(![EXIT_OK, EXIT_ERROR, EXIT_ABSENT]
            .iter()
            .any(|c| (10..=14).contains(c)));
        // The French and English verdicts both name the rollback command.
        for l in [Lang::Fr, Lang::En] {
            let t = report(
                &Outcome::Mismatch {
                    read: vec![0; 32],
                    backup: Some("/b.json".into()),
                },
                Mode::Write,
                l,
                "x",
            )
            .0;
            assert!(t.contains("akmctl rename --device-name --restore /b.json --yes"));
        }
    }

    #[test]
    fn this_command_uses_the_production_proof_and_no_leftover_of_the_old_flow() {
        let src = include_str!("devnamecmd.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains("devname::SEQUENCE_PROOF,"));
        assert!(!prod.contains(&["SequenceProof", "::"].concat()));
        // Gone: the typed word, the name typed again, the configuration lock,
        // the power-cycle countdown.
        for gone in [
            ["ECR", "IRE"].concat(),
            ["allow_device", "_name_write"].concat(),
            ["wait_", "reconnect"].concat(),
            ["config", "::load"].concat(),
            ["fn ", "sleep"].concat(),
        ] {
            assert!(!prod.contains(&gone), "{gone} is back");
        }
        // This file never writes a file but the backup (through akm-core).
        assert!(
            !prod.contains("fs::write")
                && !prod.contains("OpenOptions")
                && !prod.contains("config::save")
        );
        // The MTU comes from the read-only `inspect` verb, never from a dry run
        // of a HID_CONTROL byte; and the only pkexec call is in hid_control.rs.
        assert!(prod.contains("inspect_control_mtu("));
        assert!(!prod.contains("HidControlOp"));
        assert!(!prod.contains("pkexec\""));
        // No fixed MTU is assumed anywhere in this command.
        assert!(
            !prod.contains("Some(672)")
                && !prod.contains("Some(66)")
                && !prod.contains("Some(185)")
        );
    }
}
