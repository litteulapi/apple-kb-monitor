//! Keyboard LEDs: state from sysfs, control through keyd's virtual keyboard.
//!
//! keyd grabs the physical keyboard (EVIOCGRAB), and the kernel then drops
//! EV_LED events injected through another handle (input_inject_event). keyd
//! forwards EV_LED received on its *virtual keyboard* to every grabbed device,
//! so that is the write target when keyd runs; otherwise the Apple evdev is
//! written directly. Apple keyboards are recognised by HID_ID (vendor/product),
//! never by the user-editable device name.

use crate::model::model_from_uevent;
use std::path::{Path, PathBuf};

/// Name keyd gives its uinput keyboard.
pub const KEYD_VIRTUAL_KEYBOARD: &str = "keyd virtual keyboard";

/// Why an LED write did not happen. Never silent: callers get the reason.
#[derive(Debug)]
pub enum LedError {
    /// Neither keyd's virtual keyboard nor an Apple evdev was found.
    NoTarget,
    /// The evdev could not be opened for writing (permissions: needs the uaccess ACL).
    Open(PathBuf, std::io::Error),
    /// The write failed or was short.
    Write(PathBuf, std::io::Error),
}

impl std::fmt::Display for LedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedError::NoTarget => write!(f, "no keyd virtual keyboard nor Apple evdev found"),
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
    /// Apple keyboard evdev, written directly (keyd not running).
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

/// True if this sysfs input/led node belongs to a supported Apple keyboard.
fn node_is_apple_keyboard(node: &Path) -> bool {
    hid_uevent_above(node).is_some_and(|u| model_from_uevent(&u).is_some())
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

/// Apple keyboard evdev, recognised by HID_ID of its HID parent.
pub fn find_apple_evdev_in(sys: &Path, dev: &Path) -> Option<PathBuf> {
    class_entries(&sys.join("class/input"))
        .into_iter()
        .find_map(|(name, p)| {
            (name.starts_with("event") && node_is_apple_keyboard(&p))
                .then(|| dev.join("input").join(&name))
        })
}

/// Pick the LED write target: keyd virtual keyboard first, Apple evdev as fallback.
pub fn led_target_in(sys: &Path, dev: &Path) -> Option<LedTarget> {
    find_keyd_virtual_evdev_in(sys, dev)
        .map(LedTarget::KeydVirtual)
        .or_else(|| find_apple_evdev_in(sys, dev).map(LedTarget::AppleDirect))
}

/// brightness file of the Apple keyboard's own `input*::<suffix>` LED
/// (`suffix` = "capslock" / "numlock"), not just any LED of that name.
pub fn apple_led_brightness_in(sys: &Path, suffix: &str) -> Option<PathBuf> {
    let want = format!("::{}", suffix);
    class_entries(&sys.join("class/leds"))
        .into_iter()
        .find_map(|(name, p)| {
            (name.ends_with(&want) && node_is_apple_keyboard(&p.join("device")))
                .then(|| p.join("brightness"))
        })
}

fn read_led_in(sys: &Path, suffix: &str) -> bool {
    apple_led_brightness_in(sys, suffix)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|v| v.trim() != "0")
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

fn write_led_event(target: &Path, led: u16, value: bool) -> Result<(), LedError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .map_err(|e| LedError::Open(target.to_path_buf(), e))?;
    f.write_all(&build_led_event(led, value))
        .map_err(|e| LedError::Write(target.to_path_buf(), e))
}

/// Set a keyboard LED (0=NumLock, 1=CapsLock, 2=ScrollLock) through keyd's
/// virtual keyboard, or directly on the Apple evdev if keyd is absent.
/// Returns the target used, or an explicit error (never a silent no-op).
pub fn set_led(led: u16, value: bool) -> Result<LedTarget, LedError> {
    let target = led_target_in(Path::new("/sys"), Path::new("/dev")).ok_or(LedError::NoTarget)?;
    write_led_event(target.path(), led, value)?;
    Ok(target)
}

/// Flash CapsLock LED N times (for notifications). Stops and logs on the first error.
pub fn flash_capslock(times: u8) {
    std::thread::spawn(move || {
        // Restore the Apple keyboard's real CapsLock state afterwards.
        let was_on = read_led_state().0;
        for _ in 0..times {
            for v in [!was_on, was_on] {
                if let Err(e) = set_led(1, v) {
                    eprintln!("[keyboard] LED flash aborted: {}", e);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    });
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
            "Clavier de maria #1",
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

    #[test]
    fn led_target_prefers_keyd_virtual_keyboard() {
        let f = Fake::new();
        f.add_hid_keyboard(
            "0005:05AC:0256.0014",
            "0005:000005AC:00000256",
            "input7",
            "event7",
            "Clavier de maria #1",
        );
        let dev = Path::new("/dev");
        assert_eq!(
            led_target_in(&f.0, dev),
            Some(LedTarget::AppleDirect(PathBuf::from("/dev/input/event7")))
        );
        // keyd appears
        f.put(
            "devices/virtual/input/input99/name",
            "keyd virtual keyboard\n",
        );
        std::fs::create_dir_all(f.0.join("devices/virtual/input/input99/event99")).unwrap();
        symlink(
            f.0.join("devices/virtual/input/input99/event99"),
            f.0.join("class/input/event99"),
        )
        .unwrap();
        symlink(
            f.0.join("devices/virtual/input/input99"),
            f.0.join("devices/virtual/input/input99/event99/device"),
        )
        .unwrap();
        assert_eq!(
            led_target_in(&f.0, dev),
            Some(LedTarget::KeydVirtual(PathBuf::from("/dev/input/event99")))
        );
    }

    #[test]
    fn led_target_none_without_keyboard() {
        let f = Fake::new();
        assert_eq!(led_target_in(&f.0, Path::new("/dev")), None);
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
