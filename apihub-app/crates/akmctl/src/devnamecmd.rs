//! `akmctl rename --device-name ...`: the name stored IN the keyboard.

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use akm_core::devname::{
    self, Backup, Frame, NameDoor, Outcome, Preflight, PreflightFail, RenameEnv, Request,
};
use akm_core::parity::FeatureSink;
use akm_core::registry::WriteSession;
use akm_core::{tr, Snapshot};

use crate::bus;
use crate::cli::{EXIT_ABSENT, EXIT_ERROR, EXIT_OK};

pub const EXIT_NO_CONFIRM: u8 = 10;
pub const EXIT_PREFLIGHT: u8 = 11;
pub const EXIT_CANCELLED: u8 = 12;
pub const EXIT_UNVERIFIED: u8 = 13;
pub const EXIT_MISMATCH: u8 = 14;
pub const EXIT_WRITE_UNCERTAIN: u8 = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show,
    Rename { name: String, mode: Mode },
    Restore { file: PathBuf, mode: Mode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Validate, show the bytes, pre-flight without MTU, backup.
    DryRun,
    /// The whole pre-flight and the backup; stops where the confirmation would be asked.
    Check,
    /// The write (the default).
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Opts {
    pub yes: bool,
    pub verbose: bool,
}

fn two_names() -> String {
    tr!("Two names exist:\n\
  * alias on THIS computer (BlueZ Alias): `akmctl rename <name>`, the default, no risk, nothing written into the keyboard;\n\
  * name stored IN the keyboard (firmware, reports 0x51-0x54 / 0x55), seen by every host: `akmctl rename --device-name`.\n")
}

pub trait Io {
    fn interactive(&self) -> bool;
    fn read_line(&mut self) -> Option<String>;
    fn out(&mut self, s: &str);
    fn journal(&mut self, s: &str);
}

pub trait World {
    fn snapshot(&mut self) -> Result<Snapshot, (u8, String)>;
    fn doctor_green(&mut self, mac: Option<&str>) -> bool;
    fn probe_control_mtu(&mut self, mac: &str) -> Result<(u16, String), String>;
    /// The write door of keyboard `mac`, never another keyboard's.
    fn open_door(&mut self, mac: &str) -> Result<Box<dyn NameDoor>, String>;
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf>;
    fn now_unix(&mut self) -> u64;
    fn reread_name(&mut self) -> Result<(bool, String), String>;
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
    fn open_door(&mut self, mac: &str) -> Result<Box<dyn NameDoor>, String> {
        akm_core::hidraw::WriteDoor::open_for(mac).map(|d| Box::new(d) as Box<dyn NameDoor>)
    }
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
        devname::write_backup(&devname::state_dir(), b)
    }
    fn now_unix(&mut self) -> u64 {
        now_unix()
    }
    fn reread_name(&mut self) -> Result<(bool, String), String> {
        let conn = bus::connect().map_err(|e| e.to_string())?;
        bus::reread_name(&conn).map_err(|e| e.to_string())
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn cached_raw(s: &Snapshot) -> Option<Vec<u8>> {
    s.keyboard
        .as_ref()
        .and_then(|k| k.device.name_on_keyboard_hex.as_deref())
        .and_then(devname::unhex)
        .filter(|r| r.len() == devname::MAX_NAME_LEN)
}

pub(crate) fn snapshot_mac(s: &Snapshot) -> Option<String> {
    s.mac()
        .map(str::to_string)
        .or_else(|| s.keyboard.as_ref().and_then(|k| k.device.mac.clone()))
}

/// The refusal when `--mac` is not the keyboard whose state and name the daemon reports.
fn other_than_daemon(mac: &str, s: &Snapshot) -> Option<String> {
    match snapshot_mac(s) {
        Some(d) if d.eq_ignore_ascii_case(mac) => None,
        d => Some(tr!(
            "Refused: --mac {mac} is not the keyboard the daemon follows ({daemon}), so its pre-flight state and current name are unknown. Nothing was touched.\n",
            mac = mac,
            daemon = d.unwrap_or_else(|| tr!("none"))
        )),
    }
}

pub fn preflight_from(s: &Snapshot, doctor_green: bool, control_mtu: Option<u16>) -> Preflight {
    let k = s.keyboard.as_ref();
    Preflight {
        connected: s.connected,
        battery_pct: s.battery_pct(),
        // GET Input 0x30 of this acquisition only; a kept value may be stale.
        battery_state_normal: k
            .filter(|k| !k.battery.kept)
            .and_then(|k| k.battery.state)
            .map(|st| st == 0),
        breaker_open: k.is_some_and(|k| k.breaker_open),
        recent_read_failure: k.is_some_and(|k| k.incomplete) || s.kb_error.is_some(),
        doctor_green,
        control_mtu,
    }
}

pub fn run(action: Action, mac: Option<String>, opts: Opts) -> u8 {
    let mut world = RealWorld;
    let mut io = Stdio;
    dispatch(action, mac, opts, &mut world, &mut io)
}

pub fn dispatch(
    action: Action,
    mac: Option<String>,
    opts: Opts,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    match action {
        Action::Show => show(mac.as_deref(), world, io),
        Action::Rename { name, mode } => rename(&name, mode, mac, opts, world, io),
        Action::Restore { file, mode } => restore(&file, mode, mac, opts, world, io),
    }
}

fn show(mac: Option<&str>, world: &mut dyn World, io: &mut dyn Io) -> u8 {
    let s = match world.snapshot() {
        Ok(s) => s,
        Err((code, m)) => {
            io.journal(&format!("akmctl: {m}"));
            return code;
        }
    };
    if let Some(t) = mac.and_then(|m| other_than_daemon(m, &s)) {
        io.out(&t);
        return EXIT_ERROR;
    }
    io.out(&two_names());
    io.out(&format!(
        "{} {}\n",
        tr!("Alias (this computer):"),
        s.alias().unwrap_or(&tr!("(none)"))
    ));
    let raw = cached_raw(&s);
    let name = s
        .keyboard
        .as_ref()
        .and_then(|k| k.device.name_on_keyboard.clone())
        .or_else(|| raw.as_deref().map(devname::display_name_from_raw));
    if let Some(n) = name {
        io.out(&format!(
            "{} {n}\n",
            tr!("Stored in the keyboard (0x51-0x54, daemon cache):")
        ));
        if let Some(h) = raw {
            io.out(&format!("  {} {}\n", tr!("bytes:"), devname::hex(&h)));
        }
        EXIT_OK
    } else {
        io.out(&tr!("Stored in the keyboard: not read yet in this connection (the daemon reads 0x51-0x54 once per connection; no new hardware read is made here)\n"));
        EXIT_ERROR
    }
}

/// The detail shown by `--verbose` and `--dry-run`.
fn proof_state() -> String {
    let mut s = format!(
        "{} {}.\n{}\n",
        tr!("Proof of the sequence:"),
        devname::SEQUENCE_PROOF,
        tr!("Settled by disassembly (docs/RENAME-KEYBOARD.md):")
    );
    for u in devname::RESOLVED_BY_DISASSEMBLY {
        let _ = writeln!(s, "  - {u}");
    }
    s.push_str(&tr!("Measured on the keyboard (firmware 0x0050):\n"));
    for u in devname::RESOLVED_BY_MEASUREMENT {
        let _ = writeln!(s, "  - {u}");
    }
    s.push_str(&tr!("Not measured:\n"));
    for u in devname::UNKNOWNS {
        let _ = writeln!(s, "  - {u}");
    }
    s.push_str(&tr!("Experiments:\n"));
    for e in devname::VALIDATION_EXPERIMENTS {
        let _ = writeln!(s, "  - {e}");
    }
    s
}

fn frames_text(frames: &[Frame]) -> String {
    format!(
        "{}\n{}",
        tr!("Frame sent (the one of Lion 10.7.5 setDeviceName:, established by disassembly):"),
        devname::render_frames(frames)
    )
}

fn fail_text(f: &PreflightFail) -> String {
    f.describe()
}

fn preflight_ok_line(p: &Preflight, with_mtu: bool) -> String {
    let min = devname::MIN_CONTROL_MTU;
    let mtu = p.control_mtu.filter(|_| with_mtu);
    match (p.battery_pct, mtu) {
        (Some(b), Some(m)) => tr!(
            "Pre-flight: ok (doctor green, battery {battery} %, breaker closed, outgoing MTU {mtu} >= {min})\n",
            battery = format!("{b:.0}"),
            mtu = m,
            min = min
        ),
        (Some(b), None) => tr!(
            "Pre-flight: ok (doctor green, battery {battery} %, breaker closed)\n",
            battery = format!("{b:.0}")
        ),
        (None, Some(m)) => tr!(
            "Pre-flight: ok (doctor green, breaker closed, outgoing MTU {mtu} >= {min})\n",
            mtu = m,
            min = min
        ),
        (None, None) => tr!("Pre-flight: ok (doctor green, breaker closed)\n"),
    }
}

pub fn restore_command(backup: &Path) -> String {
    format!(
        "akmctl rename --device-name --restore {} --yes",
        backup.display()
    )
}

fn refuse_no_confirmation(io: &mut dyn Io) -> u8 {
    io.out(&tr!("Refused: no terminal to confirm. Add --yes to write without the question. Nothing was touched.\n"));
    EXIT_NO_CONFIRM
}

fn rename(
    name: &str,
    mode: Mode,
    mac: Option<String>,
    opts: Opts,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    let frames = match devname::prepare(name) {
        Ok(f) => f,
        Err(e) => {
            io.journal(&format!("akmctl: {} {e}", tr!("name refused:")));
            return EXIT_ERROR;
        }
    };
    match mode {
        Mode::DryRun => dry_run(name, &frames, mac.as_deref(), world, io),
        Mode::Check | Mode::Write => guarded(
            &Request::Rename(name.to_string()),
            &frames,
            mode,
            mac,
            opts,
            world,
            io,
        ),
    }
}

fn dry_run(
    name: &str,
    frames: &[Frame],
    mac: Option<&str>,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    io.out(&two_names());
    io.out(&format!(
        "\n{} {name:?}\n{}",
        tr!("New name stored in the keyboard:"),
        frames_text(frames)
    ));
    io.out(&tr!("\nDRY RUN: nothing is written to the keyboard.\n"));
    match world.snapshot() {
        Ok(s) => {
            if let Some(t) = mac.and_then(|m| other_than_daemon(m, &s)) {
                io.out(&t);
                return EXIT_ERROR;
            }
            let green = world.doctor_green(mac);
            let p = preflight_from(&s, green, None);
            let fails: Vec<_> = devname::preflight(&p)
                .into_iter()
                .filter(|f| !f.is_mtu())
                .collect();
            if fails.is_empty() {
                io.out(&preflight_ok_line(&p, false));
            }
            for f in &fails {
                io.out(&format!(
                    "{} {}\n",
                    tr!("Pre-flight: FAILED -"),
                    fail_text(f)
                ));
            }
            io.out(&tr!(
                "Pre-flight: control-channel MTU not probed in a dry run (needs pkexec); with --check or when writing it is probed and must be >= {min}\n",
                min = devname::MIN_CONTROL_MTU
            ));
            match cached_raw(&s) {
                Some(raw) => {
                    let m = snapshot_mac(&s).unwrap_or_else(|| "unknown".into());
                    let now = world.now_unix();
                    match Backup::new(&m, &raw, now, "daemon-cache")
                        .map_err(|e| e.clone())
                        .and_then(|b| world.save_backup(&b).map_err(|e| e.to_string()))
                    {
                        Ok(p) => io.out(&format!(
                            "{} {} (0600)\n",
                            tr!("Backup of the current name:"),
                            p.display()
                        )),
                        Err(e) => io.out(&format!(
                            "{} {e}\n",
                            tr!("Backup: FAILED -")
                        )),
                    }
                }
                None => io.out(&tr!("Backup: not possible yet (the daemon has not read 0x51-0x54 in this connection)\n")),
            }
        }
        Err((_, m)) => io.out(&format!(
            "{} ({m})\n",
            tr!("Pre-flight: daemon unavailable")
        )),
    }
    io.out(&proof_state());
    io.out(&tr!("\nTo write: the same command without --dry-run (one confirmation [y/N], or --yes). To check everything without writing: --check.\n"));
    EXIT_OK
}

fn restore(
    file: &Path,
    mode: Mode,
    mac: Option<String>,
    opts: Opts,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    let refused = |io: &mut dyn Io, e: &str| {
        io.journal(&format!("akmctl: {} {e}", tr!("backup refused:")));
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
    io.out(&tr!(
        "Backup {file}: name {name}, keyboard {mac}, saved at {when}\n",
        file = file.display(),
        name = format!("{:?}", b.display_name()),
        mac = b.mac,
        when = devname::utc_stamp(b.created_unix)
    ));
    if mode == Mode::DryRun {
        io.out(&frames_text(&frames));
        io.out(&tr!("DRY RUN: nothing written. Without --dry-run, the backup is written back into the keyboard (one confirmation [y/N], or --yes).\n"));
        return EXIT_OK;
    }
    guarded(&Request::Restore(b), &frames, mode, mac, opts, world, io)
}

struct Flow<'a> {
    world: &'a mut dyn World,
    io: &'a mut dyn Io,
    mode: Mode,
    opts: Opts,
    mac: Option<String>,
    target: String,
    door: Option<Box<dyn NameDoor>>,
    mtu: Option<Result<u16, String>>,
    probes: u32,
    preflights: u32,
    /// Why the target's hidraw node did not open.
    door_error: Option<String>,
}

struct Closed;

impl FeatureSink for Closed {
    fn set_feature(&self, _op: akm_core::registry::WriteOp, _report: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "no hidraw write door: nothing written",
        ))
    }
}

static CLOSED: Closed = Closed;

fn is_yes(answer: &str) -> bool {
    let a = answer.trim().to_lowercase();
    matches!(a.as_str(), "y" | "yes")
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
                self.io
                    .journal(&format!("akmctl: {} {m}", tr!("daemon unavailable:")));
                None
            }
        }
    }

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
                "[devname] reading the outgoing MTU of the L2CAP control channel to {mac} (pkexec akm-helper hid-inspect --mac {mac}: read-only, nothing sent)"
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
                    self.io.journal(&format!(
                        "akmctl: {} {e}",
                        tr!("control-channel MTU unknown:")
                    ));
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
                .and_then(|k| k.device.name_on_keyboard.clone())
                .or_else(|| {
                    cached_raw(&s)
                        .as_deref()
                        .map(devname::display_name_from_raw)
                });
            self.io.out(&format!(
                "{} {} → “{}”\n",
                tr!("Name stored in the keyboard:"),
                match current {
                    Some(n) => format!("“{n}”"),
                    None => tr!("(not read yet)"),
                },
                self.target
            ));
        }
        let mtu = self.control_mtu(snapshot_mac(&s).as_deref());
        let mac = self.mac.clone();
        let green = self.world.doctor_green(mac.as_deref());
        let p = preflight_from(&s, green, mtu);
        if self.preflights == 1 && devname::preflight(&p).is_empty() {
            let line = preflight_ok_line(&p, true);
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
            tr!("Backup of the current name:"),
            p.display()
        ));
        Ok(p)
    }
    fn confirm(&mut self, target: &str) -> bool {
        if self.mode == Mode::Check {
            return false;
        }
        if !self.opts.yes {
            self.io.out(&tr!(
                "Write “{target}” into the keyboard's memory? [y/N] ",
                target = target
            ));
            let answer = self.io.read_line().unwrap_or_default();
            if !is_yes(&answer) {
                return false;
            }
        }
        let mac = RenameEnv::mac(self);
        match self.world.open_door(&mac) {
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
        match self.door.take() {
            Some(d) => d.read_name(),
            None => Err(std::io::Error::other("no hidraw door")),
        }
    }
    fn log(&mut self, line: &str) {
        self.verbose(line);
    }
}

fn guarded(
    req: &Request,
    frames: &[Frame],
    mode: Mode,
    mac: Option<String>,
    opts: Opts,
    world: &mut dyn World,
    io: &mut dyn Io,
) -> u8 {
    if mode == Mode::Write && !opts.yes && !io.interactive() {
        return refuse_no_confirmation(io);
    }
    let target = match req {
        Request::Rename(n) => n.clone(),
        Request::Restore(b) => b.display_name(),
    };
    if let (Some(m), Ok(s)) = (mac.as_deref(), world.snapshot()) {
        if let Some(t) = other_than_daemon(m, &s) {
            io.out(&t);
            return EXIT_ERROR;
        }
    }
    if opts.verbose {
        io.out(&frames_text(frames));
        io.out(&proof_state());
    }
    let mut flow = Flow {
        world,
        io,
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
    let o = devname::run(req, &mut session, &mut flow);
    debug_assert!(flow.probes <= 1, "one pkexec per command");
    flow.door = None;
    if let (Outcome::WriteFailed(_), Some(d)) = (&o, &flow.door_error) {
        // The door never opened: the closed sink refused, nothing left akmctl.
        flow.io.out(&tr!(
            "Stopped: the hidraw node did not open ({d}). Nothing was written.\n",
            d = d
        ));
        return EXIT_ERROR;
    }
    if o.wrote() {
        match flow.world.reread_name() {
            Ok((true, text)) => flow.verbose(&format!("[devname] RereadName: {text}")),
            Ok((false, text)) => flow.io.journal(&format!(
                "akmctl: {} {text}",
                tr!("RereadName refused by the daemon:")
            )),
            Err(e) => flow.verbose(&format!("[devname] RereadName not delivered: {e}")),
        }
    }
    let (text, code) = report(&o, mode, &target);
    flow.io.out(&text);
    code
}

fn after_write_note() -> String {
    tr!("  BlueZ may show the old name until a later connection; persistence across a battery change is not measured.\n")
}

#[allow(clippy::too_many_lines)] // one arm per outcome
pub fn report(o: &Outcome, mode: Mode, target: &str) -> (String, u8) {
    match o {
        Outcome::Preflight(fails) => {
            let mut s = tr!("Pre-flight refused, nothing was written:\n");
            for f in fails {
                let _ = writeln!(s, "  - {}", fail_text(f));
            }
            (s, EXIT_PREFLIGHT)
        }
        Outcome::OtherKeyboard { backup, target } => (
            tr!(
                "Refused: this backup is of keyboard {backup}, the target keyboard is {target}. Nothing was touched.\n",
                backup = backup,
                target = target
            ),
            EXIT_ERROR,
        ),
        Outcome::Cancelled if mode == Mode::Check => (
            tr!("✓ --check: pre-flight green, backup made, nothing was written, nothing was asked.\n"),
            EXIT_OK,
        ),
        Outcome::Cancelled => (
            tr!("Cancelled: nothing was written into the keyboard.\n"),
            EXIT_CANCELLED,
        ),
        Outcome::Verified { .. } => (
            format!(
                "✓ {} “{target}”\n{}",
                tr!("Name written into the keyboard and read back identical:"),
                after_write_note()
            ),
            EXIT_OK,
        ),
        Outcome::Mismatch { read, backup } => {
            let mut s = format!(
                "✗ {} {} ({})\n",
                tr!("Written, but the name read back differs:"),
                match devname::name_from_raw(read) {
                    Some(n) => format!("“{n}”"),
                    None => tr!("(not printable)"),
                },
                devname::hex(read)
            );
            match backup {
                Some(b) => { let _ = writeln!(s,
                    "  {} {}",
                    tr!("Rollback:"),
                    restore_command(b)
                ); },
                None => s.push_str(&tr!("  Rollback: run --restore again with the original backup.\n")),
            }
            (s, EXIT_MISMATCH)
        }
        Outcome::Unverified { backup, error } => {
            let mut s = format!(
                "✎ {} ({error}).\n  {} akmctl rename --device-name --show\n",
                tr!("Name written, but the read-back was not possible"),
                tr!("Check later:")
            );
            if let Some(b) = backup {
                let _ = writeln!(s,
                    "  {} {}",
                    tr!("Rollback if needed:"),
                    restore_command(b)
                );
            }
            s.push_str(&after_write_note());
            (s, EXIT_UNVERIFIED)
        }
        Outcome::NoCachedName => (
            tr!("Stopped: the daemon has not read the current name (0x51-0x54) in this connection yet, no backup is possible. Try again in a moment. Nothing was written.\n"),
            EXIT_ERROR,
        ),
        Outcome::WriteFailed(e) => (
            format!(
                "✎ {} ({e}). {}\n  {} akmctl rename --device-name --show\n",
                tr!("Write uncertain: the write failed on its way, not retried"),
                tr!("A frame may have been sent: the name stored in the keyboard is not known."),
                tr!("Check in a moment:")
            ),
            EXIT_WRITE_UNCERTAIN,
        ),
        Outcome::InvalidName(e) => (
            tr!("Stopped: name or backup refused ({e}). Nothing was written.\n", e = e),
            EXIT_ERROR,
        ),
        Outcome::BackupFailed(e) => (
            tr!(
                "Stopped: the current name could not be saved ({e}). Nothing was written.\n",
                e = e
            ),
            EXIT_ERROR,
        ),
        Outcome::Disconnected => (
            tr!("Stopped: the keyboard disconnected before the write. Nothing was written.\n"),
            EXIT_ERROR,
        ),
        Outcome::Refused(e) => (
            tr!(
                "Stopped: the write was refused before sending ({e}). Nothing was written.\n",
                e = e
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
    const OLD_HEX: &str = concat!(
        "416c696365277320",
        "6b6579626f617264",
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
        k.device.name = Some("Alice's keyboard #1".into());
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

    #[derive(Clone)]
    enum Back {
        Echo,
        Hex(String),
        Error(String),
    }

    struct FakeDoor {
        writes: Writes,
        events: Events,
        back: Back,
        mac: String,
        write_fails: bool,
    }

    impl FeatureSink for FakeDoor {
        fn set_feature(&self, op: WriteOp, report: &[u8]) -> std::io::Result<()> {
            self.writes.borrow_mut().push((op, report.to_vec()));
            self.events
                .borrow_mut()
                .push(format!("write {:#04x}", report[0]));
            if self.write_fails {
                return Err(std::io::Error::from_raw_os_error(libc::EIO));
            }
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
        fn mac(&self) -> String {
            self.mac.clone()
        }
    }

    #[allow(clippy::struct_excessive_bools)] // independent switches of the simulated world
    struct FakeWorld {
        connected: bool,
        raw_hex: Option<String>,
        doctor: bool,
        mtu: Result<(u16, String), String>,
        probes: u32,
        door_fails: bool,
        door_mac: String,
        opened_for: Option<String>,
        write_fails: bool,
        reread: (bool, String),
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
                door_mac: MAC.into(),
                opened_for: None,
                write_fails: false,
                reread: (true, "name read again now (0x51-0x54 only)".into()),
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
        fn open_door(&mut self, mac: &str) -> Result<Box<dyn NameDoor>, String> {
            self.events.borrow_mut().push("door".into());
            self.opened_for = Some(mac.into());
            if self.door_fails {
                return Err(format!("no hidraw node of keyboard {mac}: nothing written"));
            }
            devname::check_door_mac(&self.door_mac, mac)?;
            Ok(Box::new(FakeDoor {
                writes: self.writes.clone(),
                events: self.events.clone(),
                back: self.back.clone(),
                mac: self.door_mac.clone(),
                write_fails: self.write_fails,
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
        fn reread_name(&mut self) -> Result<(bool, String), String> {
            self.events.borrow_mut().push("RereadName".into());
            Ok(self.reread.clone())
        }
    }

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
            "Alice's keyboard #1"
        );
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
                    &mut w,
                    &mut io,
                );
                std::fs::remove_dir_all(&dir).unwrap();
                c
            } else {
                write("Desk", ASK, &mut w, &mut io)
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
            assert_eq!(w.backups[0].name.as_deref(), Some("Alice's keyboard #1"));
            assert_eq!(io.lines.len(), 1, "{name}: nothing is asked with --yes");
            assert!(!io.out.contains("[y/N]"), "{name}");
            assert!(io.out.contains("read back identical"), "{name}");
        }
    }

    #[test]
    fn one_question_and_a_yes_writes_a_no_writes_nothing() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["y", "left"]);
        assert_eq!(write("Desk", ASK, &mut w, &mut io), EXIT_OK);
        assert_eq!(io.out.matches("[y/N]").count(), 1, "ONE question");
        assert_eq!(io.lines.len(), 1, "one line read");
        assert_eq!(w.writes.borrow().len(), 1);
        let want = format!(
            "Name stored in the keyboard: “Alice's keyboard #1” → “Desk”\n\
             Pre-flight: ok (doctor green, battery 99 %, breaker closed, outgoing MTU 185 >= 66)\n\
             Backup of the current name: {BACKUP}\n\
             Write “Desk” into the keyboard's memory? [y/N] \
             ✓ Name written into the keyboard and read back identical: “Desk”\n  \
             BlueZ may show the old name until a later connection; persistence across a battery change is not measured.\n"
        );
        assert_eq!(io.out, want);
        assert!(
            io.journal.is_empty(),
            "no journal without --verbose: {:?}",
            io.journal
        );
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
                write("Desk", ASK, &mut w, &mut io),
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
        for answer in ["y", "yes", "Y"] {
            let mut w = FakeWorld::green();
            let mut io = FakeIo::new(true, &[answer]);
            let code = dispatch(
                Action::Rename {
                    name: "Desk".into(),
                    mode: Mode::Write,
                },
                Some(MAC.into()),
                ASK,
                &mut w,
                &mut io,
            );
            assert_eq!(code, EXIT_OK, "{answer}");
            assert!(io
                .out
                .contains("Write “Desk” into the keyboard's memory? [y/N]"));
            assert!(io
                .out
                .contains("✓ Name written into the keyboard and read back identical: “Desk”"));
            assert!(
                io.out.contains("Pre-flight: ok") && io.out.contains("Backup of the current name:")
            );
            assert!(io.out.contains("battery change"));
        }
        assert!(is_yes(" Yes \n") && !is_yes("o") && !is_yes("oui") && !is_yes("non"));
    }

    #[test]
    fn read_back_different_is_14_with_the_exact_rollback_command() {
        let mut w = FakeWorld::green();
        w.back = Back::Hex(OLD_HEX.into());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_MISMATCH);
        assert_eq!(w.writes.borrow().len(), 1, "no retry");
        assert_eq!(
            acts(&w).iter().filter(|e| *e == "read back").count(),
            1,
            "one read-back, no retry"
        );
        assert!(io
            .out
            .contains("the name read back differs: “Alice's keyboard #1”"));
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
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_UNVERIFIED);
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
                    name: "Desk".into(),
                    mode: Mode::Check,
                },
                None,
                opts,
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
        let mut w = FakeWorld::green();
        w.mtu = Ok((48, String::new()));
        let mut io = FakeIo::new(true, &[]);
        assert_eq!(check(&mut w, &mut io, ASK), EXIT_PREFLIGHT);
        assert_eq!(acts(&w), vec!["pkexec"]);
        assert!(io.out.contains("Pre-flight refused") && io.out.contains("48"));
        let mut w = FakeWorld::green();
        w.mtu = Err("authentication dismissed".into());
        let mut io = FakeIo::new(true, &[]);
        assert_eq!(check(&mut w, &mut io, ASK), EXIT_PREFLIGHT);
        assert!(io
            .journal
            .iter()
            .any(|l| l.contains("authentication dismissed")));
    }

    #[test]
    fn pre_flight_refusals_list_every_reason_and_write_nothing() {
        let mut w = FakeWorld::green();
        w.connected = false;
        w.doctor = false;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_PREFLIGHT);
        assert_eq!(acts(&w), vec!["pkexec"], "no backup, no door, no write");
        assert!(io.out.contains("keyboard not connected") && io.out.contains("doctor"));
        let mut w = FakeWorld::green();
        w.raw_hex = None;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_ERROR);
        assert!(w.writes.borrow().is_empty() && io.out.contains("Nothing was written"));
        let mut w = FakeWorld::green();
        w.door_fails = true;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_ERROR);
        assert!(w.writes.borrow().is_empty());
        assert!(io.out.contains("Nothing was written"));
        assert!(!acts(&w).contains(&"RereadName".to_string()));
    }

    #[test]
    fn the_pre_flight_line_is_one_sentence_per_variant() {
        let mut p = preflight_from(&snap(true, Some(OLD_HEX)), true, Some(185));
        assert_eq!(
            preflight_ok_line(&p, false),
            "Pre-flight: ok (doctor green, battery 99 %, breaker closed)\n"
        );
        p.battery_pct = None;
        assert_eq!(
            preflight_ok_line(&p, true),
            "Pre-flight: ok (doctor green, breaker closed, outgoing MTU 185 >= 66)\n"
        );
        assert_eq!(
            preflight_ok_line(&p, false),
            "Pre-flight: ok (doctor green, breaker closed)\n"
        );
    }

    #[test]
    fn a_mac_other_than_the_daemon_keyboard_is_refused_before_anything() {
        const OTHER: &str = "11:22:33:44:55:66";
        let dir = std::env::temp_dir().join(format!("akm-devnamecmd-m-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let b = Backup::new(OTHER, &devname::unhex(OLD_HEX).unwrap(), 1, "daemon-cache").unwrap();
        let file = devname::write_backup(&dir, &b).unwrap();
        let rename = |mode| Action::Rename {
            name: "New".into(),
            mode,
        };
        let restore = Action::Restore {
            file,
            mode: Mode::Write,
        };
        for action in [
            Action::Show,
            rename(Mode::Write),
            rename(Mode::Check),
            rename(Mode::DryRun),
            restore,
        ] {
            let mut w = FakeWorld::green();
            w.door_mac = OTHER.into();
            let mut io = FakeIo::new(false, &[]);
            let code = dispatch(action, Some(OTHER.into()), YES, &mut w, &mut io);
            assert_eq!(code, EXIT_ERROR);
            assert!(acts(&w).is_empty(), "{:?}", acts(&w));
            assert!(w.backups.is_empty() && w.writes.borrow().is_empty());
            assert!(io
                .out
                .contains("is not the keyboard the daemon follows (AA:BB:CC:DD:EE:F1)"));
            assert!(!io.out.contains("Pre-flight: ok"), "{}", io.out);
            assert!(!io.out.contains("Alias (this computer)"), "{}", io.out);
        }
        let _ = std::fs::remove_dir_all(&dir);
        let mut io = FakeIo::new(false, &[]);
        let mut w = FakeWorld::green();
        let code = dispatch(Action::Show, Some(MAC.to_lowercase()), YES, &mut w, &mut io);
        assert_eq!(code, EXIT_OK, "{}", io.out);
        let mut s = snap(true, Some(OLD_HEX));
        assert!(other_than_daemon(&MAC.to_lowercase(), &s).is_none());
        s.keyboard = None;
        assert!(other_than_daemon(MAC, &s).is_some());
    }

    #[test]
    fn the_door_is_the_target_s_and_never_another_keyboard_s() {
        for explicit in [None, Some(MAC.to_string())] {
            let mut w = FakeWorld::green();
            w.door_mac = "AA:BB:CC:DD:EE:02".into();
            let mut io = FakeIo::new(false, &[]);
            let code = dispatch(
                Action::Rename {
                    name: "Desk".into(),
                    mode: Mode::Write,
                },
                explicit,
                YES,
                &mut w,
                &mut io,
            );
            assert_eq!(code, EXIT_ERROR);
            assert!(
                w.writes.borrow().is_empty(),
                "no frame for the other keyboard"
            );
            let a = acts(&w);
            assert!(a.contains(&"door".to_string()), "{a:?}");
            assert!(!a.iter().any(|e| e.starts_with("write")), "{a:?}");
            assert!(!a.contains(&"RereadName".to_string()));
            assert_eq!(w.opened_for.as_deref(), Some(MAC));
            assert!(
                io.out.contains("the hidraw node did not open")
                    && io.out.contains("AA:BB:CC:DD:EE:02")
                    && io.out.contains("Nothing was written"),
                "{}",
                io.out
            );
        }
        let mut w = FakeWorld::green();
        w.door_mac = String::new();
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_ERROR);
        assert!(w.writes.borrow().is_empty());
    }

    #[test]
    fn a_failed_write_through_the_door_is_uncertain_not_unwritten() {
        let mut w = FakeWorld::green();
        w.write_fails = true;
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_WRITE_UNCERTAIN);
        assert_eq!(EXIT_WRITE_UNCERTAIN, 15);
        let a = acts(&w);
        assert_eq!(
            a,
            vec!["pkexec", "backup", "door", "write 0x55", "RereadName"],
            "one write, no retry, no read-back, the daemon re-reads"
        );
        assert!(io.out.contains("Write uncertain"), "{}", io.out);
        assert!(io.out.contains("A frame may have been sent"));
        assert!(!io.out.contains("Nothing was written"));
        assert!(io.out.contains("--show"));
    }

    #[test]
    fn the_daemons_answer_to_reread_name_is_reported() {
        let mut w = FakeWorld::green();
        w.reread = (false, "no keyboard connected: nothing to read again".into());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_OK);
        assert!(
            io.journal
                .iter()
                .any(|l| l.contains("RereadName refused") && l.contains("no keyboard connected")),
            "{:?}",
            io.journal
        );
        let mut w = FakeWorld::green();
        w.reread = (
            true,
            "name read again in 21 s (0x51-0x54 only, at most once per 30 s)".into(),
        );
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_OK);
        assert!(!io.journal.iter().any(|l| l.contains("RereadName")));
        let verbose = Opts {
            yes: true,
            verbose: true,
        };
        let mut w = FakeWorld::green();
        w.reread = (true, "name read again in 21 s".into());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", verbose, &mut w, &mut io), EXIT_OK);
        assert!(io
            .journal
            .iter()
            .any(|l| l.contains("RereadName: name read again in 21 s")));
    }

    #[test]
    fn a_keyboard_named_with_an_accent_is_shown_saved_and_renamed() {
        let mut raw = "Zoë's Keyboard #1".as_bytes().to_vec();
        raw.resize(devname::MAX_NAME_LEN, 0);
        let hex = raw.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        let mut w = FakeWorld::green();
        w.raw_hex = Some(hex.clone());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_OK, "{}", io.out);
        assert_eq!(w.backups.len(), 1);
        assert_eq!(w.backups[0].name, None);
        assert_eq!(w.backups[0].raw().unwrap(), raw);
        assert!(
            io.out.contains("“Zoë's Keyboard #1” → “Desk”"),
            "{}",
            io.out
        );
        let mut w = FakeWorld::green();
        w.raw_hex = Some(hex);
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(dispatch(Action::Show, None, YES, &mut w, &mut io), EXIT_OK);
        assert!(io.out.contains("Zoë's Keyboard #1"), "{}", io.out);
    }

    #[test]
    fn dry_run_never_probes_asks_nor_writes_and_shows_the_bytes() {
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(true, &["y"]);
        let code = dispatch(
            Action::Rename {
                name: "Desk".into(),
                mode: Mode::DryRun,
            },
            None,
            ASK,
            &mut w,
            &mut io,
        );
        assert_eq!(code, EXIT_OK);
        assert_eq!(w.probes, 0);
        assert_eq!(acts(&w), vec!["backup"]);
        assert_eq!(io.lines.len(), 1);
        for s in [
            "DRY RUN",
            "55 44 65 73 6b",
            "wire   : 53 55",
            "U4 [measured 2026-10-02]",
            "U5 [measured 2026-10-02]",
            "U3 NOT MEASURED",
            "Two names exist",
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
        assert_eq!(write("Desk", opts, &mut w, &mut io), EXIT_OK);
        assert!(io.out.contains("wire   : 53 55 44 65 73 6b"));
        assert!(io.out.contains("Proof of the sequence"));
        for s in [
            "[devname] pre-flight ok",
            "[devname] control-channel outgoing MTU = 185",
            "[devname] backup written",
            "[devname] write DeviceName Feature 0x55, 65 bytes",
            "[devname] read back 0x51-0x54: 44 65 73 6b 00",
            "[devname] verified",
        ] {
            assert!(
                io.journal.iter().any(|l| l.contains(s)),
                "missing {s:?}: {:?}",
                io.journal
            );
        }
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(write("Desk", YES, &mut w, &mut io), EXIT_OK);
        assert!(!io.out.contains("wire") && !io.out.contains("Proof"));
        assert!(io.journal.is_empty(), "{:?}", io.journal);
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
                w,
                io,
            )
        };
        let bureau = devname::hex(&devname::expected_readback(
            &devname::prepare("Desk").unwrap(),
        ))
        .replace(' ', "");
        let mut w = FakeWorld::green();
        w.raw_hex = Some(bureau.clone());
        let mut io = FakeIo::new(false, &[]);
        assert_eq!(restore(&mut w, &mut io, YES), EXIT_OK);
        assert_eq!(
            acts(&w),
            vec![
                "pkexec",
                "backup",
                "door",
                "write 0x55",
                "read back",
                "RereadName"
            ]
        );
        assert_eq!(&w.writes.borrow()[0].1[1..33], &old[..]);
        assert!(io
            .out
            .contains("read back identical: “Alice's keyboard #1”"));
        assert_eq!(w.backups.len(), 1, "the current name is saved first");
        let mut w = FakeWorld::green();
        w.raw_hex = Some(bureau);
        let mut io = FakeIo::new(true, &["n"]);
        assert_eq!(restore(&mut w, &mut io, ASK), EXIT_CANCELLED);
        assert_eq!(io.out.matches("[y/N]").count(), 1);
        assert!(w.writes.borrow().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn low_percentage_passes_only_with_a_fresh_normal_battery_state() {
        let fails = |state: Option<u8>, kept: bool| {
            let mut s = snap(true, Some(OLD_HEX));
            let b = &mut s.keyboard.as_mut().unwrap().battery;
            b.percentage_fine = Some(5.0);
            b.state = state;
            b.kept = kept;
            devname::preflight(&preflight_from(&s, true, Some(185)))
        };
        assert!(!fails(Some(0), false).contains(&PreflightFail::Battery));
        assert!(fails(Some(1), false).contains(&PreflightFail::Battery));
        assert!(fails(Some(0), true).contains(&PreflightFail::Battery));
        assert!(fails(None, false).contains(&PreflightFail::Battery));
    }

    #[test]
    fn restore_refuses_a_backup_of_another_keyboard() {
        let dir = std::env::temp_dir().join(format!("akm-devnamecmd-o-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = devname::unhex(OLD_HEX).unwrap();
        let b = Backup::new("11:22:33:44:55:66", &old, 1, "daemon-cache").unwrap();
        let file = devname::write_backup(&dir, &b).unwrap();
        let mut w = FakeWorld::green();
        let mut io = FakeIo::new(false, &[]);
        let code = dispatch(
            Action::Restore {
                file,
                mode: Mode::Write,
            },
            None,
            YES,
            &mut w,
            &mut io,
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(code, EXIT_ERROR);
        assert!(
            acts(&w).is_empty(),
            "no pkexec, no backup, no door: {:?}",
            acts(&w)
        );
        assert!(w.writes.borrow().is_empty());
        assert!(io.out.contains("keyboard 11:22:33:44:55:66"));
        assert!(io.out.contains(MAC));
    }

    #[test]
    fn every_outcome_has_its_exit_code() {
        let r = |o: &Outcome, m: Mode| report(o, m, "x").1;
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
            Outcome::NoCachedName,
            Outcome::Disconnected,
            Outcome::Refused("r".into()),
            Outcome::InvalidName("n".into()),
            Outcome::BackupFailed("No space left on device".into()),
        ] {
            assert_eq!(r(&o, Mode::Write), EXIT_ERROR, "{o:?}");
            let t = report(&o, Mode::Write, "x").0;
            assert!(t.ends_with("Nothing was written.\n"), "{t}");
            assert!(!t.contains(&format!("{o:?}")), "no Debug output: {t}");
        }
        assert_eq!(
            r(&Outcome::WriteFailed("w".into()), Mode::Write),
            EXIT_WRITE_UNCERTAIN
        );
        assert_eq!(
            [
                EXIT_NO_CONFIRM,
                EXIT_PREFLIGHT,
                EXIT_CANCELLED,
                EXIT_UNVERIFIED,
                EXIT_MISMATCH,
                EXIT_WRITE_UNCERTAIN
            ],
            [10, 11, 12, 13, 14, 15]
        );
        assert!(![EXIT_OK, EXIT_ERROR, EXIT_ABSENT]
            .iter()
            .any(|c| (10..=15).contains(c)));
        {
            let t = report(
                &Outcome::Mismatch {
                    read: vec![0; 32],
                    backup: Some("/b.json".into()),
                },
                Mode::Write,
                "x",
            )
            .0;
            assert!(t.contains("akmctl rename --device-name --restore /b.json --yes"));
        }
    }

    #[test]
    fn stderr_lines_are_translated() {
        let prod = akm_core::srclint::prod_tokens(include_str!("devnamecmd.rs"));
        let lines: Vec<&str> = prod.split("journal(&format!(\"akmctl:").skip(1).collect();
        assert!(lines.len() >= 4);
        for l in lines {
            assert!(
                l.starts_with('{'),
                "untranslated: {}",
                &l[..l.len().min(60)]
            );
        }
    }

    #[test]
    fn this_command_runs_the_guarded_flow_and_no_leftover_of_the_old_flow() {
        let prod = akm_core::srclint::prod_tokens(include_str!("devnamecmd.rs"));
        assert!(prod.contains("devname::run(req,&mutsession,&mutflow)"));
        for gone in [
            ["ECR", "IRE"].concat(),
            ["allow_device", "_name_write"].concat(),
            ["wait_", "reconnect"].concat(),
            ["config", "::load"].concat(),
            ["fn", "sleep("].concat(),
        ] {
            assert!(!prod.contains(&gone), "{gone} is back");
        }
        // This file never writes a file but the backup (through akm-core).
        assert!(
            !prod.contains("fs::write")
                && !prod.contains("OpenOptions")
                && !prod.contains("config::save")
        );
        // The MTU comes from the read-only `inspect` verb, never from a dry run of a HID_CONTROL
        assert!(prod.contains("inspect_control_mtu("));
        assert!(!prod.contains("HidControlOp"));
        assert!(!prod.contains("pkexec\""));
        assert!(
            !prod.contains("Some(672)")
                && !prod.contains("Some(66)")
                && !prod.contains("Some(185)")
        );
    }
}
