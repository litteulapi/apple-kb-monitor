//! Keyboard LEDs: state from sysfs, control through evdev EV_LED.
//!
//! keyd grabs the keyboards listed in its configuration (EVIOCGRAB), and the
//! kernel then drops EV_LED events injected through another handle
//! (input_inject_event). keyd forwards EV_LED received on its *virtual
//! keyboard* to every device it grabbed, so that is the write target **only
//! when keyd really grabs this Apple keyboard** (its `[ids]` match the
//! keyboard's vendor:product); otherwise the Apple evdev is written directly
//! (#125). Apple keyboards are recognised by HID_ID (vendor/product), never by
//! the user-editable device name.
//!
//! The input core ignores an EV_LED equal to the device's current LED state.
//! keyd's virtual keyboard may disagree with the Apple keyboard, so a write
//! through keyd is primed with the Apple keyboard's real state first.
//!
//! NumLock is never switched on: on the BCM2042 keyboards hid-apple then
//! turns J/K/L/U/I/O/M... into keypad keys (`APPLE_NUMLOCK_EMULATION`).

use crate::model::{mac_from_uevent, model_from_uevent, parse_hid_id};
use std::path::{Path, PathBuf};

/// Name keyd gives its uinput keyboard.
pub const KEYD_VIRTUAL_KEYBOARD: &str = "keyd virtual keyboard";
/// keyd's configuration directory (read only, never written).
pub const KEYD_CONFIG_DIR: &str = "/etc/keyd";

/// LED codes (`LED_*` of `input-event-codes.h`).
pub const LED_NUML: u16 = 0;
pub const LED_CAPSL: u16 = 1;
pub const LED_SCROLLL: u16 = 2;
pub const LED_COMPOSE: u16 = 3;
pub const LED_KANA: u16 = 4;

/// sysfs suffix (`inputN::<suffix>`) of an LED code.
pub fn led_suffix(led: u16) -> Option<&'static str> {
    Some(match led {
        LED_NUML => "numlock",
        LED_CAPSL => "capslock",
        LED_SCROLLL => "scrolllock",
        LED_COMPOSE => "compose",
        LED_KANA => "kana",
        _ => return None,
    })
}

/// Why an LED write did not happen. Never silent: callers get the reason.
#[derive(Debug)]
pub enum LedError {
    /// No Apple keyboard evdev was found.
    NoTarget,
    /// Unknown LED code.
    Unsupported(u16),
    /// Switching NumLock on is refused (hid-apple NumLock emulation).
    NumLockForbidden,
    /// The evdev could not be opened for writing (permissions: needs the uaccess ACL).
    Open(PathBuf, std::io::Error),
    /// The write failed or was short.
    Write(PathBuf, std::io::Error),
}

impl std::fmt::Display for LedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedError::NoTarget => write!(f, "no Apple keyboard evdev found"),
            LedError::Unsupported(l) => write!(f, "unsupported LED code {}", l),
            LedError::NumLockForbidden => write!(
                f,
                "refusing to switch NumLock on (hid-apple would turn letters into keypad keys)"
            ),
            LedError::Open(p, e) => write!(f, "cannot open {}: {}", p.display(), e),
            LedError::Write(p, e) => write!(f, "cannot write {}: {}", p.display(), e),
        }
    }
}

impl std::error::Error for LedError {}

/// Where an LED event goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedTarget {
    /// keyd's virtual keyboard: keyd relays EV_LED to the grabbed keyboard.
    KeydVirtual(PathBuf),
    /// Apple keyboard evdev, written directly (not grabbed by keyd).
    AppleDirect(PathBuf),
}

impl LedTarget {
    pub fn path(&self) -> &Path {
        match self {
            LedTarget::KeydVirtual(p) | LedTarget::AppleDirect(p) => p,
        }
    }
}

/// Walk up from a sysfs node to the HID device and return its uevent text.
fn hid_uevent_above(node: &Path) -> Option<String> {
    let real = std::fs::canonicalize(node).ok()?;
    for dir in real.ancestors().skip(1) {
        if let Ok(u) = std::fs::read_to_string(dir.join("uevent")) {
            if u.lines().any(|l| l.starts_with("HID_ID=")) {
                return Some(u);
            }
        }
    }
    None
}

/// HID uevent of the supported Apple keyboard owning this node (and with
/// this MAC when one is given).
fn apple_keyboard_uevent(node: &Path, mac: Option<&str>) -> Option<String> {
    let u = hid_uevent_above(node)?;
    model_from_uevent(&u)?;
    match mac {
        Some(m) => mac_from_uevent(&u)
            .is_some_and(|um| um.eq_ignore_ascii_case(m))
            .then_some(u),
        None => Some(u),
    }
}

/// Sorted `name -> path` entries of a sysfs class directory.
fn class_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    v.sort();
    v
}

/// keyd's virtual keyboard evdev (`/dev/input/eventN`), if keyd is running.
pub fn find_keyd_virtual_evdev_in(sys: &Path, dev: &Path) -> Option<PathBuf> {
    class_entries(&sys.join("class/input"))
        .into_iter()
        .find_map(|(name, p)| {
            if !name.starts_with("event") {
                return None;
            }
            let n = std::fs::read_to_string(p.join("device/name")).ok()?;
            (n.trim() == KEYD_VIRTUAL_KEYBOARD).then(|| dev.join("input").join(&name))
        })
}

/// Apple keyboard evdev (with this MAC if given) and its (vendor, product).
pub fn find_apple_evdev_for_in(
    sys: &Path,
    dev: &Path,
    mac: Option<&str>,
) -> Option<(PathBuf, u32, u32)> {
    class_entries(&sys.join("class/input"))
        .into_iter()
        .find_map(|(name, p)| {
            if !name.starts_with("event") {
                return None;
            }
            let u = apple_keyboard_uevent(&p, mac)?;
            let (vid, pid) = parse_hid_id(&u)?;
            Some((dev.join("input").join(&name), vid, pid))
        })
}

/// First Apple keyboard evdev, recognised by HID_ID of its HID parent.
pub fn find_apple_evdev_in(sys: &Path, dev: &Path) -> Option<PathBuf> {
    find_apple_evdev_for_in(sys, dev, None).map(|(p, _, _)| p)
}

/// Does one keyd configuration text grab `vid:pid`? Follows keyd's `[ids]`
/// rules: explicit ids, or `*` with `-id` exclusions; `k:` / `m:` prefixes
/// (keyboard / mouse only) and a trailing third field are accepted.
pub fn keyd_conf_matches(conf: &str, vid: u32, pid: u32) -> bool {
    let want = format!("{:04x}:{:04x}", vid, pid);
    let mut in_ids = false;
    let (mut wildcard, mut listed, mut excluded) = (false, false, false);
    for line in conf.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        if l.starts_with('[') {
            in_ids = l.eq_ignore_ascii_case("[ids]");
            continue;
        }
        if !in_ids {
            continue;
        }
        let (neg, id) = match l.strip_prefix('-') {
            Some(r) => (true, r.trim()),
            None => (false, l),
        };
        if id == "*" {
            wildcard |= !neg;
            continue;
        }
        let id = id.strip_prefix("k:").unwrap_or(id);
        if id.starts_with("m:") {
            continue; // mice only
        }
        let mut f = id.split(':');
        let (Some(v), Some(p)) = (f.next(), f.next()) else {
            continue;
        };
        if format!("{}:{}", v, p).eq_ignore_ascii_case(&want) {
            if neg {
                excluded = true;
            } else {
                listed = true;
            }
        }
    }
    listed || (wildcard && !excluded)
}

/// Does keyd's configuration grab `vid:pid`? `None` if the directory cannot
/// be read (then nothing is known).
pub fn keyd_grabs_in(conf_dir: &Path, vid: u32, pid: u32) -> Option<bool> {
    let rd = std::fs::read_dir(conf_dir).ok()?;
    Some(rd.flatten().any(|e| {
        let p = e.path();
        p.extension().is_some_and(|x| x == "conf")
            && std::fs::read_to_string(&p).is_ok_and(|c| keyd_conf_matches(&c, vid, pid))
    }))
}

/// Pick the LED write target for the Apple keyboard (with this MAC if
/// given): keyd's virtual keyboard only if keyd runs **and** grabs this
/// keyboard (or its configuration cannot be read), its own evdev otherwise.
pub fn led_target_for_in(
    sys: &Path,
    dev: &Path,
    keyd_conf: &Path,
    mac: Option<&str>,
) -> Option<LedTarget> {
    let (apple, vid, pid) = find_apple_evdev_for_in(sys, dev, mac)?;
    match find_keyd_virtual_evdev_in(sys, dev) {
        Some(v) if keyd_grabs_in(keyd_conf, vid, pid) != Some(false) => {
            Some(LedTarget::KeydVirtual(v))
        }
        _ => Some(LedTarget::AppleDirect(apple)),
    }
}

/// [`led_target_for_in`] for the first Apple keyboard.
pub fn led_target_in(sys: &Path, dev: &Path, keyd_conf: &Path) -> Option<LedTarget> {
    led_target_for_in(sys, dev, keyd_conf, None)
}

/// brightness file of the Apple keyboard's own `input*::<suffix>` LED
/// (`suffix` = "capslock" / "numlock"), not just any LED of that name.
pub fn apple_led_brightness_for_in(sys: &Path, suffix: &str, mac: Option<&str>) -> Option<PathBuf> {
    let want = format!("::{}", suffix);
    class_entries(&sys.join("class/leds"))
        .into_iter()
        .find_map(|(name, p)| {
            (name.ends_with(&want) && apple_keyboard_uevent(&p.join("device"), mac).is_some())
                .then(|| p.join("brightness"))
        })
}

/// [`apple_led_brightness_for_in`] for the first Apple keyboard.
pub fn apple_led_brightness_in(sys: &Path, suffix: &str) -> Option<PathBuf> {
    apple_led_brightness_for_in(sys, suffix, None)
}

fn read_led_for_in(sys: &Path, suffix: &str, mac: Option<&str>) -> bool {
    apple_led_brightness_for_in(sys, suffix, mac)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|v| v.trim() != "0")
}

fn read_led_in(sys: &Path, suffix: &str) -> bool {
    read_led_for_in(sys, suffix, None)
}

/// Read CapsLock and NumLock LED state of the Apple keyboard from sysfs.
/// Returns (capslock_on, numlock_on); false if the Apple keyboard has no such LED.
pub fn read_led_state() -> (bool, bool) {
    let sys = Path::new("/sys");
    (read_led_in(sys, "capslock"), read_led_in(sys, "numlock"))
}

/// `struct input_event` (x86_64: 24 bytes) carrying EV_LED.
pub fn build_led_event(led: u16, value: bool) -> [u8; 24] {
    let mut ev = [0u8; 24]; // tv_sec(8) + tv_usec(8) + type(2) + code(2) + value(4)
    ev[16..18].copy_from_slice(&0x11u16.to_ne_bytes()); // EV_LED
    ev[18..20].copy_from_slice(&led.to_ne_bytes());
    ev[20..24].copy_from_slice(&(value as i32).to_ne_bytes());
    ev
}

/// Write EV_LED events, in order, on one open handle.
fn write_led_events(target: &Path, led: u16, values: &[bool]) -> Result<(), LedError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .map_err(|e| LedError::Open(target.to_path_buf(), e))?;
    for &v in values {
        f.write_all(&build_led_event(led, v))
            .map_err(|e| LedError::Write(target.to_path_buf(), e))?;
    }
    Ok(())
}

#[cfg(test)]
fn write_led_event(target: &Path, led: u16, value: bool) -> Result<(), LedError> {
    write_led_events(target, led, &[value])
}

/// The EV_LED values to write to reach `value` on the Apple keyboard whose
/// LED is `current`. Nothing if already there. Through keyd, prime the
/// virtual keyboard with `current` first: if its own state already equals
/// `value` the input core would drop the write; the priming event is relayed
/// as a no-op since the Apple keyboard is at `current`.
pub fn led_writes(target: &LedTarget, current: bool, value: bool) -> Vec<bool> {
    match (current == value, target) {
        (true, _) => Vec::new(),
        (false, LedTarget::KeydVirtual(_)) => vec![current, value],
        (false, LedTarget::AppleDirect(_)) => vec![value],
    }
}

/// Validate an LED request: known code, never NumLock on.
pub fn check_led(led: u16, value: bool) -> Result<&'static str, LedError> {
    let suffix = led_suffix(led).ok_or(LedError::Unsupported(led))?;
    if led == LED_NUML && value {
        return Err(LedError::NumLockForbidden);
    }
    Ok(suffix)
}

fn set_led_in(
    sys: &Path,
    dev: &Path,
    keyd_conf: &Path,
    mac: Option<&str>,
    led: u16,
    value: bool,
) -> Result<LedTarget, LedError> {
    let suffix = check_led(led, value)?;
    let target = led_target_for_in(sys, dev, keyd_conf, mac).ok_or(LedError::NoTarget)?;
    let current = read_led_for_in(sys, suffix, mac);
    let writes = led_writes(&target, current, value);
    if !writes.is_empty() {
        write_led_events(target.path(), led, &writes)?;
    }
    Ok(target)
}

/// Set an LED of the Apple keyboard with this MAC (first one if `None`).
/// [`LED_NUML`] can only be switched off. Returns the target used, or an
/// explicit error (never a silent no-op).
pub fn set_led_for(mac: Option<&str>, led: u16, value: bool) -> Result<LedTarget, LedError> {
    set_led_in(
        Path::new("/sys"),
        Path::new("/dev"),
        Path::new(KEYD_CONFIG_DIR),
        mac,
        led,
        value,
    )
}

/// Set an LED (see [`set_led_for`]) of the first Apple keyboard.
pub fn set_led(led: u16, value: bool) -> Result<LedTarget, LedError> {
    set_led_for(None, led, value)
}

/// Flash the CapsLock LED of the keyboard with this MAC (first one if
/// `None`) N times. Stops and logs on the first error.
pub fn flash_capslock_for(mac: Option<String>, times: u8) {
    std::thread::spawn(move || {
        let sys = Path::new("/sys");
        // Restore the Apple keyboard's real CapsLock state afterwards.
        let was_on = read_led_for_in(sys, "capslock", mac.as_deref());
        for _ in 0..times {
            for v in [!was_on, was_on] {
                if let Err(e) = set_led_for(mac.as_deref(), LED_CAPSL, v) {
                    eprintln!("[keyboard] LED flash aborted: {}", e);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    });
}

/// Flash CapsLock LED N times on the first Apple keyboard.
pub fn flash_capslock(times: u8) {
    flash_capslock_for(None, times)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU32, Ordering};

    static T: AtomicU32 = AtomicU32::new(0);

    struct Fake(PathBuf);
    impl Fake {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "kbled-test-{}-{}",
                std::process::id(),
                T.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Fake(p)
        }
        fn put(&self, rel: &str, c: &str) {
            let f = self.0.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, c).unwrap();
        }
        /// HID device + input node + evdev + capslock LED, wired with symlinks like real sysfs.
        fn add_hid_keyboard(&self, hid: &str, hid_id: &str, input: &str, event: &str, name: &str) {
            let hdir = format!("devices/virtual/uhid/{hid}");
            self.put(
                &format!("{hdir}/uevent"),
                &format!("HID_ID={hid_id}\nHID_NAME={name}\n"),
            );
            self.put(&format!("{hdir}/input/{input}/name"), name);
            self.put(&format!("{hdir}/input/{input}/{event}/dev"), "13:64\n");
            self.put(
                &format!("{hdir}/input/{input}/{input}::capslock/brightness"),
                "1\n",
            );
            self.put(
                &format!("{hdir}/input/{input}/{input}::numlock/brightness"),
                "0\n",
            );
            std::fs::create_dir_all(self.0.join("class/input")).unwrap();
            std::fs::create_dir_all(self.0.join("class/leds")).unwrap();
            let abs = |r: &str| self.0.join(r);
            symlink(
                abs(&format!("{hdir}/input/{input}/{event}")),
                abs(&format!("class/input/{event}")),
            )
            .unwrap();
            symlink(
                abs(&format!("{hdir}/input/{input}")),
                abs(&format!("class/input/{input}")),
            )
            .unwrap();
            // event's device -> input node, as in real sysfs
            symlink(
                abs(&format!("{hdir}/input/{input}")),
                abs(&format!("{hdir}/input/{input}/{event}/device")),
            )
            .unwrap();
            for l in ["capslock", "numlock"] {
                let led = abs(&format!("{hdir}/input/{input}/{input}::{l}"));
                symlink(abs(&format!("{hdir}/input/{input}")), led.join("device")).unwrap();
                symlink(&led, abs(&format!("class/leds/{input}::{l}"))).unwrap();
            }
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn led_event_layout() {
        let ev = build_led_event(1, true);
        assert_eq!(&ev[..16], &[0u8; 16]);
        assert_eq!(u16::from_ne_bytes([ev[16], ev[17]]), 0x11);
        assert_eq!(u16::from_ne_bytes([ev[18], ev[19]]), 1);
        assert_eq!(i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]), 1);
        assert_eq!(build_led_event(0, false)[20..], [0, 0, 0, 0]);
    }

    #[test]
    fn apple_evdev_found_by_hid_id_not_by_name() {
        let f = Fake::new();
        // User-renamed Apple keyboard + a non-Apple one named like an Apple one.
        f.add_hid_keyboard(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "input7",
            "event7",
            "Clavier de alice #1",
        );
        f.add_hid_keyboard(
            "0003:046D:C31C.0001",
            "0003:0000046D:0000C31C",
            "input3",
            "event3",
            "Apple Keyboard Lookalike",
        );
        let dev = Path::new("/dev");
        assert_eq!(
            find_apple_evdev_in(&f.0, dev),
            Some(PathBuf::from("/dev/input/event7"))
        );
    }

    impl Fake {
        fn add_keyd_virtual(&self) {
            std::fs::create_dir_all(self.0.join("class/input")).unwrap();
            self.put(
                "devices/virtual/input/input99/name",
                "keyd virtual keyboard\n",
            );
            std::fs::create_dir_all(self.0.join("devices/virtual/input/input99/event99")).unwrap();
            symlink(
                self.0.join("devices/virtual/input/input99/event99"),
                self.0.join("class/input/event99"),
            )
            .unwrap();
            symlink(
                self.0.join("devices/virtual/input/input99"),
                self.0.join("devices/virtual/input/input99/event99/device"),
            )
            .unwrap();
        }
        fn set_uniq(&self, hid: &str, hid_id: &str, mac: &str) {
            self.put(
                &format!("devices/virtual/uhid/{hid}/uevent"),
                &format!("HID_ID={hid_id}\nHID_NAME=x\nHID_UNIQ={mac}\n"),
            );
        }
        fn keyd_dir(&self, conf: Option<&str>) -> PathBuf {
            let d = self.0.join("etc-keyd");
            std::fs::create_dir_all(&d).unwrap();
            if let Some(c) = conf {
                std::fs::write(d.join("apple-keyboard.conf"), c).unwrap();
            }
            d
        }
    }

    const KEYD_0256: &str = "[ids]\n05ac:0256\n\n[main]\nf3 = macro(M-z)\n";

    #[test]
    fn led_target_uses_keyd_only_when_it_grabs_the_keyboard() {
        let f = Fake::new();
        f.add_hid_keyboard(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "input7",
            "event7",
            "Clavier de alice #1",
        );
        let dev = Path::new("/dev");
        let conf = f.keyd_dir(Some(KEYD_0256));
        let direct = Some(LedTarget::AppleDirect(PathBuf::from("/dev/input/event7")));
        let keyd = Some(LedTarget::KeydVirtual(PathBuf::from("/dev/input/event99")));
        assert_eq!(led_target_in(&f.0, dev, &conf), direct);
        // keyd appears and grabs 05ac:0256
        f.add_keyd_virtual();
        assert_eq!(led_target_in(&f.0, dev, &conf), keyd);
        // keyd runs but its configuration does not list this keyboard (#125)
        std::fs::write(conf.join("apple-keyboard.conf"), "[ids]\n05ac:0255\n").unwrap();
        assert_eq!(led_target_in(&f.0, dev, &conf), direct);
        // wildcard with exclusion
        std::fs::write(conf.join("apple-keyboard.conf"), "[ids]\n*\n-05ac:0256\n").unwrap();
        assert_eq!(led_target_in(&f.0, dev, &conf), direct);
        std::fs::write(conf.join("apple-keyboard.conf"), "[ids]\n*\n").unwrap();
        assert_eq!(led_target_in(&f.0, dev, &conf), keyd);
        // unreadable configuration: keyd is running, assume it grabbed
        assert_eq!(led_target_in(&f.0, dev, &f.0.join("missing")), keyd);
    }

    #[test]
    fn led_target_none_without_keyboard() {
        let f = Fake::new();
        let conf = f.keyd_dir(None);
        assert_eq!(led_target_in(&f.0, Path::new("/dev"), &conf), None);
        // keyd alone is not a target: no Apple keyboard to relay to.
        f.add_keyd_virtual();
        assert_eq!(led_target_in(&f.0, Path::new("/dev"), &conf), None);
    }

    #[test]
    fn led_target_follows_the_requested_keyboard() {
        let f = Fake::new();
        f.add_hid_keyboard(
            "0005:004C:029C.0001",
            "0005:0000004C:0000029C",
            "input5",
            "event5",
            "Magic",
        );
        f.set_uniq(
            "0005:004C:029C.0001",
            "0005:0000004C:0000029C",
            "aa:bb:cc:dd:ee:02",
        );
        f.add_hid_keyboard(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "input7",
            "event7",
            "A1314",
        );
        f.set_uniq(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "aa:bb:cc:dd:ee:f1",
        );
        f.add_keyd_virtual();
        let conf = f.keyd_dir(Some(KEYD_0256));
        let dev = Path::new("/dev");
        assert_eq!(
            led_target_for_in(&f.0, dev, &conf, Some("AA:BB:CC:DD:EE:F1")),
            Some(LedTarget::KeydVirtual(PathBuf::from("/dev/input/event99")))
        );
        assert_eq!(
            led_target_for_in(&f.0, dev, &conf, Some("AA:BB:CC:DD:EE:02")),
            Some(LedTarget::AppleDirect(PathBuf::from("/dev/input/event5")))
        );
        assert_eq!(
            led_target_for_in(&f.0, dev, &conf, Some("11:22:33:44:55:66")),
            None
        );
    }

    #[test]
    fn keyd_ids_rules() {
        assert!(keyd_conf_matches("[ids]\n05ac:0256\n", 0x05ac, 0x0256));
        assert!(keyd_conf_matches("[ids]\nk:004c:029c\n", 0x004c, 0x029c));
        assert!(keyd_conf_matches(
            "[ids]\n05AC:0256:a1b2c3d4\n",
            0x05ac,
            0x0256
        ));
        assert!(!keyd_conf_matches("[ids]\nm:05ac:0256\n", 0x05ac, 0x0256));
        assert!(!keyd_conf_matches("[ids]\n05ac:0255\n", 0x05ac, 0x0256));
        assert!(!keyd_conf_matches("[main]\n05ac:0256\n", 0x05ac, 0x0256));
        assert!(keyd_conf_matches("[ids]\n* # all\n", 0x004c, 0x0267));
        assert!(!keyd_conf_matches("[ids]\n*\n-004c:0267\n", 0x004c, 0x0267));
        assert!(!keyd_conf_matches("", 0x05ac, 0x0256));
        let shipped = include_str!("../../../keyd/apple-keyboard.conf");
        for mi in crate::model::APPLE_MODELS {
            let vid = if mi.family == crate::model::Family::Bcm2042 {
                crate::model::APPLE_USB_VID
            } else {
                crate::model::APPLE_BT_VID
            };
            assert!(keyd_conf_matches(shipped, vid, mi.pid), "{:#06x}", mi.pid);
        }
    }

    #[test]
    fn numlock_is_never_switched_on() {
        assert!(matches!(
            check_led(LED_NUML, true),
            Err(LedError::NumLockForbidden)
        ));
        assert_eq!(check_led(LED_NUML, false).unwrap(), "numlock");
        assert_eq!(check_led(LED_CAPSL, true).unwrap(), "capslock");
        assert!(matches!(check_led(9, true), Err(LedError::Unsupported(9))));
        // set_led refuses before looking for any device.
        let f = Fake::new();
        let err = set_led_in(&f.0, Path::new("/dev"), &f.0, None, LED_NUML, true).unwrap_err();
        assert!(err.to_string().contains("NumLock"));
    }

    #[test]
    fn writes_through_keyd_are_primed_with_the_real_state() {
        let k = LedTarget::KeydVirtual(PathBuf::from("/dev/input/event99"));
        let d = LedTarget::AppleDirect(PathBuf::from("/dev/input/event7"));
        assert!(led_writes(&k, true, true).is_empty());
        assert!(led_writes(&d, false, false).is_empty());
        assert_eq!(led_writes(&k, false, true), vec![false, true]);
        assert_eq!(led_writes(&k, true, false), vec![true, false]);
        assert_eq!(led_writes(&d, false, true), vec![true]);
    }

    #[test]
    fn set_led_writes_the_events_on_the_chosen_target() {
        let f = Fake::new();
        f.add_hid_keyboard(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "input7",
            "event7",
            "A1314",
        );
        // Fake capslock is lit ("1"): switching it off through a fake /dev.
        let dev = f.0.join("dev");
        std::fs::create_dir_all(dev.join("input")).unwrap();
        std::fs::write(dev.join("input/event7"), b"").unwrap();
        let conf = f.keyd_dir(Some(KEYD_0256));
        let t = set_led_in(&f.0, &dev, &conf, None, LED_CAPSL, false).unwrap();
        assert_eq!(t, LedTarget::AppleDirect(dev.join("input/event7")));
        assert_eq!(
            std::fs::read(dev.join("input/event7")).unwrap(),
            build_led_event(LED_CAPSL, false).to_vec()
        );
        // Through keyd: primed with the current state (on), then off.
        f.add_keyd_virtual();
        std::fs::write(dev.join("input/event99"), b"").unwrap();
        let t = set_led_in(&f.0, &dev, &conf, None, LED_CAPSL, false).unwrap();
        assert_eq!(t, LedTarget::KeydVirtual(dev.join("input/event99")));
        let mut want = build_led_event(LED_CAPSL, true).to_vec();
        want.extend_from_slice(&build_led_event(LED_CAPSL, false));
        assert_eq!(std::fs::read(dev.join("input/event99")).unwrap(), want);
    }

    #[test]
    fn led_state_reads_the_apple_keyboard_led_only() {
        let f = Fake::new();
        f.add_hid_keyboard(
            "0003:046D:C31C.0001",
            "0003:0000046D:0000C31C",
            "input3",
            "event3",
            "Other",
        );
        f.put(
            "devices/virtual/uhid/0003:046D:C31C.0001/input/input3/input3::capslock/brightness",
            "1\n",
        );
        f.add_hid_keyboard(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "input7",
            "event7",
            "Apple",
        );
        f.put(
            "devices/virtual/uhid/0005:05AC:0256.0014/input/input7/input7::capslock/brightness",
            "0\n",
        );
        f.put(
            "devices/virtual/uhid/0005:05AC:0256.0014/input/input7/input7::numlock/brightness",
            "1\n",
        );
        assert!(!read_led_in(&f.0, "capslock")); // the other keyboard's lit LED is ignored
        assert!(read_led_in(&f.0, "numlock"));
    }

    #[test]
    fn write_led_event_writes_24_bytes_and_reports_errors() {
        let f = Fake::new();
        let file = f.0.join("evdev");
        std::fs::write(&file, b"").unwrap();
        write_led_event(&file, 1, true).unwrap();
        assert_eq!(
            std::fs::read(&file).unwrap(),
            build_led_event(1, true).to_vec()
        );
        let err = write_led_event(&f.0.join("missing/dir/event"), 1, true).unwrap_err();
        assert!(matches!(err, LedError::Open(..)));
        assert!(err.to_string().contains("cannot open"));
    }
}
