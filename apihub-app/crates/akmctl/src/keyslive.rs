//! `akmctl keys --live` (#113): live view of the key events of the Apple
//! keyboard — HID usage (`MSC_SCAN`), evdev code, name — one line per press,
//! repeat and release.
//!
//! Read-only: the evdev node is opened for reading only, never grabbed, and
//! nothing is sent to the keyboard. Nothing is stored: the lines go to the
//! terminal and nowhere else. Leave with Escape held 2 s, or Ctrl-C.
//!
//! When keyd holds the keyboard (`EVIOCGRAB`), the kernel hands the events of
//! the Apple node to keyd alone; keyd's virtual keyboard is then read too:
//! its lines show what keyd emits (evdev code after keyd), without HID usage.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use akm_core::{keymap, keytable, led};

use crate::{EXIT_ERROR, EXIT_OK};

/// `struct input_event` on 64-bit Linux: `timeval` (2 × i64), type, code, value.
pub const EVENT_SIZE: usize = 24;
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_MSC: u16 = 4;
const MSC_SCAN: u16 = 4;
pub const KEY_ESC: u16 = 1;
/// Escape held this long ends the view.
pub const ESC_HOLD: Duration = Duration::from_secs(2);

/// One `input_event`, without its kernel timestamp (not shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

/// Whole events of `buf`; a trailing partial event is ignored.
pub fn parse_events(buf: &[u8]) -> Vec<RawEvent> {
    buf.chunks_exact(EVENT_SIZE)
        .map(|c| RawEvent {
            kind: u16::from_ne_bytes([c[16], c[17]]),
            code: u16::from_ne_bytes([c[18], c[19]]),
            value: i32::from_ne_bytes([c[20], c[21], c[22], c[23]]),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Up,
    Down,
    Repeat,
}

/// One key event as the kernel reports it: the HID usage it announced just
/// before (`MSC_SCAN`), if any, and the evdev code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub hid: Option<u32>,
    pub code: u16,
    pub state: State,
}

/// Pairs each `EV_KEY` with the `MSC_SCAN` of its report.
#[derive(Debug, Default)]
pub struct Decoder {
    scan: Option<u32>,
}

impl Decoder {
    pub fn feed(&mut self, e: RawEvent) -> Option<KeyEvent> {
        match (e.kind, e.code) {
            (EV_MSC, MSC_SCAN) => {
                self.scan = Some(e.value as u32);
                None
            }
            (EV_KEY, code) => {
                let state = match e.value {
                    0 => State::Up,
                    1 => State::Down,
                    _ => State::Repeat,
                };
                // The kernel repeats a key without a new MSC_SCAN.
                let hid = if state == State::Repeat {
                    None
                } else {
                    self.scan.take()
                };
                Some(KeyEvent { hid, code, state })
            }
            (EV_SYN, _) => {
                self.scan = None;
                None
            }
            _ => None,
        }
    }
}

/// "Escape held 2 s": armed by a press, disarmed by the release.
#[derive(Debug, Default)]
pub struct EscHold {
    since: Option<Instant>,
}

impl EscHold {
    pub fn on(&mut self, e: &KeyEvent, now: Instant) {
        if e.code != KEY_ESC {
            return;
        }
        match e.state {
            State::Down => self.since = Some(now),
            State::Up => self.since = None,
            // a repeat without the press seen (view started mid-hold): arm now
            State::Repeat => {
                self.since.get_or_insert(now);
            }
        }
    }

    /// Time left before the hold is long enough; `None` when not armed.
    pub fn remaining(&self, now: Instant, hold: Duration) -> Option<Duration> {
        self.since
            .map(|t| hold.saturating_sub(now.saturating_duration_since(t)))
    }
}

/// `0007:0029` (usage page : usage), as HID usage tables print them.
pub fn hid_text(hid: Option<u32>) -> String {
    hid.map_or("-".into(), |h| {
        format!("{:04x}:{:04x}", h >> 16, h & 0xffff)
    })
}

/// Physical key of the Apple table for this HID usage ("F5", "Eject"...).
fn phys_of(hid: Option<u32>) -> &'static str {
    hid.and_then(|h| keytable::keys(true).into_iter().find(|k| k.scancode == h))
        .map_or("", |k| k.id)
}

pub const HEADER: &str = "source  HID usage   evdev  name                     state    key";

pub fn line(source: &str, e: &KeyEvent) -> String {
    format!(
        "{source:<7} {:<11} {:<6} {:<24} {:<8} {}",
        hid_text(e.hid),
        e.code,
        keymap::name_of(e.code).unwrap_or("(unnamed)"),
        match e.state {
            State::Down => "down",
            State::Up => "up",
            State::Repeat => "repeat",
        },
        phys_of(e.hid)
    )
    .trim_end()
    .to_string()
}

/// A node to read and the label of its lines.
pub struct Source {
    pub label: &'static str,
    pub path: PathBuf,
}

/// The Apple keyboard node, and keyd's virtual keyboard when keyd runs.
pub fn sources_in(sys: &Path, dev: &Path) -> Vec<Source> {
    let mut v = Vec::new();
    if let Some(path) = led::find_apple_evdev_in(sys, dev) {
        v.push(Source {
            label: "apple",
            path,
        });
    }
    if let Some(path) = led::find_keyd_virtual_evdev_in(sys, dev) {
        v.push(Source {
            label: "keyd",
            path,
        });
    }
    v
}

#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    /// Escape held long enough.
    EscHeld,
    /// Every node is gone (keyboard disconnected, asleep, keyd stopped).
    Gone,
}

/// Reads the nodes until Escape is held `hold` or all of them are closed.
/// One line per key event on `out`. The descriptors are only read.
pub fn watch(
    inputs: &mut [(&'static str, File)],
    out: &mut dyn Write,
    hold: Duration,
) -> io::Result<Exit> {
    let mut decoders: Vec<Decoder> = inputs.iter().map(|_| Decoder::default()).collect();
    let mut open: Vec<bool> = inputs.iter().map(|_| true).collect();
    let mut esc = EscHold::default();
    let mut buf = [0u8; EVENT_SIZE * 64];
    loop {
        if !open.iter().any(|o| *o) {
            return Ok(Exit::Gone);
        }
        let timeout = match esc.remaining(Instant::now(), hold) {
            Some(d) if d.is_zero() => return Ok(Exit::EscHeld),
            Some(d) => d.as_millis().clamp(1, i32::MAX as u128) as i32,
            None => -1,
        };
        let mut fds: Vec<libc::pollfd> = inputs
            .iter()
            .zip(&open)
            .map(|((_, f), o)| libc::pollfd {
                fd: if *o { f.as_raw_fd() } else { -1 },
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd for the call.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        for (i, p) in fds.iter().enumerate() {
            if p.revents == 0 {
                continue;
            }
            let got = match inputs[i].1.read(&mut buf) {
                Ok(0) => 0,
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue
                }
                // ENODEV: the node went away with the keyboard
                Err(_) => 0,
            };
            if got == 0 {
                open[i] = false;
                continue;
            }
            let now = Instant::now();
            for raw in parse_events(&buf[..got]) {
                if let Some(key) = decoders[i].feed(raw) {
                    esc.on(&key, now);
                    writeln!(out, "{}", line(inputs[i].0, &key))?;
                }
            }
            out.flush()?;
        }
    }
}

fn open_read_only(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Descriptor of a source, for the tests that feed a pipe.
#[cfg(test)]
fn file_from(fd: std::os::fd::RawFd) -> File {
    use std::os::fd::FromRawFd;
    // SAFETY: the test owns `fd` (one end of a pipe it just created).
    unsafe { File::from_raw_fd(fd) }
}

/// `akmctl keys --live`.
pub fn command() -> u8 {
    let sources = sources_in(Path::new("/sys"), Path::new("/dev"));
    if !sources.iter().any(|s| s.label == "apple") {
        eprintln!(
            "akmctl: no supported Apple keyboard is connected (press a key to wake it, then retry)"
        );
        return EXIT_ERROR;
    }
    let mut inputs = Vec::new();
    for s in &sources {
        match open_read_only(&s.path) {
            Ok(f) => inputs.push((s.label, f)),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "akmctl: {} is not readable by this user: {e}\n        reading key events needs the group \"input\" (sudo usermod -aG input $USER, then log in again); nothing was read",
                    s.path.display()
                );
                return EXIT_ERROR;
            }
            Err(e) => {
                eprintln!("akmctl: {}: {e}", s.path.display());
                return EXIT_ERROR;
            }
        }
    }
    println!("Live key events, read-only, nothing is recorded. Leave: hold Escape 2 s, or Ctrl-C.");
    for s in &sources {
        println!("  {:<6} {}", s.label, s.path.display());
    }
    if sources.iter().any(|s| s.label == "keyd") {
        println!("  keyd is running: if it holds the keyboard, only \"keyd\" lines appear (what keyd emits, no HID usage)");
    }
    println!("{HEADER}");
    let stdout = io::stdout();
    match watch(&mut inputs, &mut stdout.lock(), ESC_HOLD) {
        Ok(Exit::EscHeld) => {
            println!("Escape held: end of the view.");
            EXIT_OK
        }
        Ok(Exit::Gone) => {
            println!("The keyboard is gone (disconnected or asleep): end of the view.");
            EXIT_OK
        }
        // stdout closed (`| head`): a normal end
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => EXIT_OK,
        Err(e) => {
            eprintln!("akmctl: {e}");
            EXIT_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
        let mut b = [0u8; EVENT_SIZE];
        b[16..18].copy_from_slice(&kind.to_ne_bytes());
        b[18..20].copy_from_slice(&code.to_ne_bytes());
        b[20..24].copy_from_slice(&value.to_ne_bytes());
        b
    }

    /// What the kernel emits for one HID key: MSC_SCAN, EV_KEY, SYN_REPORT.
    fn key(hid: u32, code: u16, value: i32) -> Vec<u8> {
        [
            ev(EV_MSC, MSC_SCAN, hid as i32),
            ev(EV_KEY, code, value),
            ev(EV_SYN, 0, 0),
        ]
        .concat()
    }

    fn pipe() -> (File, File) {
        let mut fds = [0 as std::os::fd::RawFd; 2];
        // SAFETY: `fds` is a valid array of two descriptors to fill.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (file_from(fds[0]), file_from(fds[1]))
    }

    #[test]
    fn events_are_decoded_with_their_hid_usage() {
        let mut bytes = key(0x7_0004, 30, 1);
        bytes.extend(ev(EV_KEY, 30, 2)); // kernel autorepeat: no MSC_SCAN
        bytes.extend(ev(EV_SYN, 0, 0));
        bytes.extend(key(0x7_0004, 30, 0));
        bytes.extend(key(0xc_00b8, 161, 1)); // Eject, consumer page
        bytes.extend([1, 2, 3]); // a partial event is dropped
        let mut d = Decoder::default();
        let got: Vec<KeyEvent> = parse_events(&bytes)
            .into_iter()
            .filter_map(|e| d.feed(e))
            .collect();
        assert_eq!(
            got,
            [
                KeyEvent {
                    hid: Some(0x7_0004),
                    code: 30,
                    state: State::Down
                },
                KeyEvent {
                    hid: None,
                    code: 30,
                    state: State::Repeat
                },
                KeyEvent {
                    hid: Some(0x7_0004),
                    code: 30,
                    state: State::Up
                },
                KeyEvent {
                    hid: Some(0xc_00b8),
                    code: 161,
                    state: State::Down
                },
            ]
        );
        // a scan code never leaks from one report to the next
        let mut d = Decoder::default();
        assert_eq!(
            d.feed(RawEvent {
                kind: EV_MSC,
                code: MSC_SCAN,
                value: 0x7_0004
            }),
            None
        );
        assert_eq!(
            d.feed(RawEvent {
                kind: EV_SYN,
                code: 0,
                value: 0
            }),
            None
        );
        assert_eq!(
            d.feed(RawEvent {
                kind: EV_KEY,
                code: 30,
                value: 1
            })
            .unwrap()
            .hid,
            None
        );
        // other event types (LED, relative...) give no line
        assert_eq!(
            d.feed(RawEvent {
                kind: 0x11,
                code: 1,
                value: 1
            }),
            None
        );
    }

    #[test]
    fn a_line_shows_hid_usage_evdev_code_and_name() {
        let l = line(
            "apple",
            &KeyEvent {
                hid: Some(0x7_0064),
                code: 86,
                state: State::Down,
            },
        );
        assert_eq!(
            l,
            "apple   0007:0064   86     KEY_102ND                down     NonUS"
        );
        let l = line(
            "apple",
            &KeyEvent {
                hid: Some(0x7_0035),
                code: 41,
                state: State::Up,
            },
        );
        assert!(
            l.contains("0007:0035")
                && l.contains(" 41 ")
                && l.contains("KEY_GRAVE")
                && l.ends_with("Grave"),
            "{l}"
        );
        let l = line(
            "keyd",
            &KeyEvent {
                hid: None,
                code: 30,
                state: State::Repeat,
            },
        );
        assert_eq!(
            l,
            "keyd    -           30     KEY_A                    repeat"
        );
        assert!(line(
            "apple",
            &KeyEvent {
                hid: None,
                code: 0x2ff,
                state: State::Down
            }
        )
        .contains("(unnamed)"));
        assert!(HEADER.starts_with("source  HID usage   evdev  name"));
    }

    #[test]
    fn escape_must_stay_down_for_the_whole_hold() {
        let t0 = Instant::now();
        let hold = Duration::from_secs(2);
        let esc = |state| KeyEvent {
            hid: Some(0x7_0029),
            code: KEY_ESC,
            state,
        };
        let mut h = EscHold::default();
        assert_eq!(h.remaining(t0, hold), None);
        h.on(
            &KeyEvent {
                hid: None,
                code: 30,
                state: State::Down,
            },
            t0,
        );
        assert_eq!(h.remaining(t0, hold), None, "another key does not arm it");
        h.on(&esc(State::Down), t0);
        assert_eq!(
            h.remaining(t0 + Duration::from_millis(500), hold),
            Some(Duration::from_millis(1500))
        );
        h.on(&esc(State::Repeat), t0 + Duration::from_millis(600));
        assert_eq!(
            h.remaining(t0 + Duration::from_secs(1), hold),
            Some(Duration::from_secs(1)),
            "a repeat does not restart the hold"
        );
        assert_eq!(
            h.remaining(t0 + Duration::from_secs(3), hold),
            Some(Duration::ZERO)
        );
        h.on(&esc(State::Up), t0 + Duration::from_secs(1));
        assert_eq!(
            h.remaining(t0 + Duration::from_secs(9), hold),
            None,
            "a short press is just a key"
        );
    }

    #[test]
    fn watch_prints_each_key_and_ends_when_escape_is_held() {
        let (r, mut w) = pipe();
        w.write_all(&key(0x7_0064, 86, 1)).unwrap();
        w.write_all(&key(0x7_0064, 86, 0)).unwrap();
        // a short Escape: not an exit
        w.write_all(&key(0x7_0029, KEY_ESC, 1)).unwrap();
        w.write_all(&key(0x7_0029, KEY_ESC, 0)).unwrap();
        // Escape pressed and never released; the writer stays open
        w.write_all(&key(0x7_0029, KEY_ESC, 1)).unwrap();
        let mut out = Vec::new();
        let started = Instant::now();
        let exit = watch(&mut [("apple", r)], &mut out, Duration::from_millis(150)).unwrap();
        assert_eq!(exit, Exit::EscHeld);
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "left before the hold was over"
        );
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "{text}");
        assert!(
            lines[0].contains("0007:0064")
                && lines[0].contains("KEY_102ND")
                && lines[0].contains("down")
        );
        assert!(lines[1].contains("KEY_102ND") && lines[1].contains("up"));
        assert!(lines[4].contains("KEY_ESC") && lines[4].contains("down"));
        drop(w);
    }

    #[test]
    fn watch_ends_when_the_keyboard_goes_away_and_reads_two_sources() {
        let (apple, mut wa) = pipe();
        let (keyd, mut wk) = pipe();
        wk.write_all(&[ev(EV_KEY, 30, 1), ev(EV_SYN, 0, 0)].concat())
            .unwrap();
        wa.write_all(&key(0x7_0004, 30, 1)).unwrap();
        drop(wa);
        drop(wk);
        let mut out = Vec::new();
        let exit = watch(&mut [("apple", apple), ("keyd", keyd)], &mut out, ESC_HOLD).unwrap();
        assert_eq!(exit, Exit::Gone);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("apple   0007:0004   30     KEY_A"), "{text}");
        assert!(text.contains("keyd    -           30     KEY_A"), "{text}");
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn the_apple_node_is_found_in_a_fake_sysfs_and_keyd_is_added_when_present() {
        let d = std::env::temp_dir().join(format!("akm-keyslive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let sys = d.join("sys");
        // an Apple A1314 (05ac:0256) behind its HID device, another keyboard, keyd
        let hid = sys.join("devices/hid0");
        std::fs::create_dir_all(hid.join("input/input7/event5")).unwrap();
        std::fs::write(hid.join("uevent"), "HID_ID=0005:000005AC:00000256\nHID_NAME=Clavier sans fil de alice\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n").unwrap();
        let other = sys.join("devices/hid1");
        std::fs::create_dir_all(other.join("input/input8/event6")).unwrap();
        std::fs::write(
            other.join("uevent"),
            "HID_ID=0003:0000046D:0000C31C\nHID_NAME=Other\n",
        )
        .unwrap();
        let virt = sys.join("devices/virtual/input/input9");
        std::fs::create_dir_all(virt.join("event7")).unwrap();
        std::fs::write(virt.join("name"), "keyd virtual keyboard\n").unwrap();
        let class = sys.join("class/input");
        std::fs::create_dir_all(&class).unwrap();
        let link = |target: PathBuf, name: &str| {
            std::os::unix::fs::symlink(target, class.join(name)).unwrap()
        };
        link(other.join("input/input8/event6"), "event6");
        link(hid.join("input/input7/event5"), "event5");
        std::os::unix::fs::symlink(
            hid.join("input/input7"),
            hid.join("input/input7/event5/device"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            other.join("input/input8"),
            other.join("input/input8/event6/device"),
        )
        .unwrap();
        let found = sources_in(&sys, Path::new("/dev"));
        assert_eq!(found.len(), 1, "keyd not running yet");
        assert_eq!(
            (found[0].label, found[0].path.as_path()),
            ("apple", Path::new("/dev/input/event5"))
        );
        link(virt.join("event7"), "event7");
        std::os::unix::fs::symlink(&virt, virt.join("event7/device")).unwrap();
        let found = sources_in(&sys, Path::new("/dev"));
        let got: Vec<_> = found
            .iter()
            .map(|s| (s.label, s.path.to_str().unwrap()))
            .collect();
        assert_eq!(
            got,
            [
                ("apple", "/dev/input/event5"),
                ("keyd", "/dev/input/event7")
            ]
        );
        // no Apple keyboard: nothing to read
        std::fs::remove_file(class.join("event5")).unwrap();
        assert!(!sources_in(&sys, Path::new("/dev"))
            .iter()
            .any(|s| s.label == "apple"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn iso_layout_decides_which_code_the_two_iso_keys_get() {
        // #90: the model `akmctl keys` shows for `iso_layout` -1 / 0 / 1, on the
        // two keys hid_apple swaps (KEY_GRAVE 41 <-> KEY_102ND 86) and no other.
        use akm_core::keymap::{effective_table, fn_table, row_note, translate, HidState};
        use std::collections::BTreeMap;
        let (grave, nd102) = (
            keymap::code_of("KEY_GRAVE").unwrap(),
            keymap::code_of("KEY_102ND").unwrap(),
        );
        assert_eq!((grave, nd102), (41, 86));
        let table = fn_table(0x0256);
        let with = |iso: i32| HidState {
            iso_layout: Some(iso),
            ..HidState::KERNEL_DEFAULT
        };
        // 1: swapped, both ways, with or without Fn
        for fn_on in [false, true] {
            assert_eq!(translate(table, &with(1), nd102, fn_on), grave);
            assert_eq!(translate(table, &with(1), grave, fn_on), nd102);
            // 0: as the keyboard reports them
            assert_eq!(translate(table, &with(0), nd102, fn_on), nd102);
            assert_eq!(translate(table, &with(0), grave, fn_on), grave);
            // -1 (auto, the kernel default): the model does not guess the
            // country code of the keyboard; shown unswapped, and flagged
            assert_eq!(translate(table, &with(-1), nd102, fn_on), nd102);
            assert_eq!(translate(table, &with(-1), grave, fn_on), grave);
        }
        // no other key moves with iso_layout
        for k in keytable::keys(true) {
            if k.code != grave && k.code != nd102 {
                assert_eq!(
                    translate(table, &with(1), k.code, false),
                    translate(table, &with(0), k.code, false),
                    "{}",
                    k.id
                );
            }
        }
        // the physical table: usage 0x64 is KEY_102ND, usage 0x35 is KEY_GRAVE
        let keys = keytable::keys(true);
        let phys = |id: &str| *keys.iter().find(|k| k.id == id).unwrap();
        assert_eq!(
            (phys("NonUS").scancode, phys("NonUS").code),
            (0x7_0064, nd102)
        );
        assert_eq!(
            (phys("Grave").scancode, phys("Grave").code),
            (0x7_0035, grave)
        );
        // and the table of `akmctl keys --all`: swapped at 1, warned at -1 only
        for (iso, nonus_code, noted) in [(1, grave, false), (0, nd102, false), (-1, nd102, true)] {
            let rows = effective_table(0x0256, &with(iso), &BTreeMap::new(), &keys);
            let row = rows.iter().find(|r| r.key.id == "NonUS").unwrap();
            assert_eq!(
                (row.plain, row.with_fn),
                (nonus_code, nonus_code),
                "iso_layout={iso}"
            );
            assert_eq!(
                row_note(row, &with(iso)).is_some_and(|n| n.contains("iso_layout=-1")),
                noted,
                "iso_layout={iso}"
            );
        }
        // what the live view prints for the proof of docs/TOUCHES.md §8
        let shown = line(
            "apple",
            &KeyEvent {
                hid: Some(0x7_0064),
                code: nd102,
                state: State::Down,
            },
        );
        assert!(
            shown.contains("0007:0064") && shown.contains("86") && shown.contains("KEY_102ND"),
            "{shown}"
        );
    }

    #[test]
    fn the_view_cannot_write_grab_or_record() {
        // Read-only by construction: the only open is `File::open`, and the
        // code before the tests has no write access, no ioctl, no file made.
        let src = include_str!("keyslive.rs");
        let prod: String = src[..src.find("#[cfg(test)]\nmod tests").unwrap()]
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "OpenOptions",
            ".write(true)",
            "File::create",
            "create_new",
            "ioctl",
            "EVIOCGRAB",
            "fs::write",
            "O_RDWR",
            "O_WRONLY",
            "uinput",
            "create_dir",
        ] {
            assert!(!prod.contains(forbidden), "{forbidden} in the live view");
        }
        assert_eq!(prod.matches("File::open(").count(), 1);
        assert_eq!(ESC_HOLD, Duration::from_secs(2));
    }
}
