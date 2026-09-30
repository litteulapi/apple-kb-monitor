//! F1/F2 brightness handler via raw evdev — replaces `apple-brightness-daemon`.
//!
//! Opens the keyd virtual keyboard evdev device, listens for
//! KEY_BRIGHTNESSDOWN (224) and KEY_BRIGHTNESSUP (225) events,
//! and adjusts monitor brightness via DDC/CI (the `ddc` module).
//!
//! Optionally sends KDE OSD notifications via `qdbus6`.

use std::ffi::CString;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;

use crate::ddc;

const KEY_BRIGHTNESSDOWN: u16 = 224;
const KEY_BRIGHTNESSUP: u16 = 225;
const EV_KEY: u16 = 1;
const INPUT_EVENT_SIZE: usize = 24; // sizeof(struct input_event) on x86_64
const STEP: i32 = 5;
/// Re-read the real brightness from the monitor if no key was pressed for this long.
const RESYNC_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

/// Cached brightness level shared across event iterations.
static BRIGHTNESS: AtomicI32 = AtomicI32::new(-1);

/// Circadian brightness curve — returns a target brightness percentage
/// based on time of day: 30% at night, ramp to 70% by 9 AM, hold,
/// ramp back down to 30% by 9 PM.
pub fn circadian_brightness() -> u16 {
    let now = unsafe {
        let epoch = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&epoch, &mut tm);
        tm
    };
    circadian_for_hour(now.tm_hour as f32 + now.tm_min as f32 / 60.0)
}

/// Pure curve behind [`circadian_brightness`]; `h` is the local hour in 0.0..24.0.
fn circadian_for_hour(h: f32) -> u16 {
    let bri = if h < 6.0 {
        30.0
    } else if h < 9.0 {
        // 6-9: ramp 30 -> 70
        30.0 + (h - 6.0) / 3.0 * 40.0
    } else if h < 17.0 {
        70.0
    } else if h < 21.0 {
        // 17-21: ramp 70 -> 30
        70.0 - (h - 17.0) / 4.0 * 40.0
    } else {
        30.0
    };
    bri.round() as u16
}

/// Spawn a background thread that listens for brightness key events
/// and adjusts the monitor via DDC/CI.
///
/// `bus` is the I2C bus path (e.g. `/dev/i2c-6`).
/// The thread is detached — it runs until the process exits.
pub fn spawn_brightness_thread(bus: String) {
    let spawned = thread::Builder::new()
        .name("brightness-evdev".into())
        .spawn(move || {
            // keyd may start after us or be restarted (ENODEV on the old node):
            // retry forever instead of silently losing the brightness keys.
            let mut last_err = String::new();
            loop {
                if let Err(e) = run_evdev_loop(&bus) {
                    if e != last_err {
                        eprintln!("[brightness] {} (retrying every 5s)", e);
                        last_err = e;
                    }
                }
                thread::sleep(std::time::Duration::from_secs(5));
            }
        });
    if let Err(e) = spawned {
        eprintln!("[brightness] cannot spawn evdev thread: {}", e);
    }
}

/// Scan `/dev/input/event*` for the keyd virtual keyboard.
fn find_keyd_device() -> Option<String> {
    let rd = std::fs::read_dir("/dev/input").ok()?;
    let mut candidates: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("event")
        })
        .map(|e| format!("/dev/input/{}", e.file_name().to_string_lossy()))
        .collect();
    candidates.sort();

    for path in &candidates {
        // Read device name via EVIOCGNAME ioctl
        let c_path = CString::new(path.as_str()).ok()?;
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            continue;
        }

        let mut name_buf = [0u8; 256];
        // EVIOCGNAME(len) = _IOC(_IOC_READ, 'E', 0x06, len)
        // _IOC_READ = 2, 'E' = 0x45
        // _IOC(2, 0x45, 0x06, 256) = (2 << 30) | (256 << 16) | (0x45 << 8) | 0x06
        const EVIOCGNAME_256: libc::c_ulong =
            (2 << 30) | (256 << 16) | (0x45 << 8) | 0x06;

        let ret = unsafe {
            libc::ioctl(fd, EVIOCGNAME_256, name_buf.as_mut_ptr())
        };
        unsafe { libc::close(fd) };

        if ret > 0 {
            let name = String::from_utf8_lossy(&name_buf[..ret as usize]);
            let name = name.trim_end_matches('\0');
            if name == "keyd virtual keyboard" {
                return Some(path.clone());
            }
        }
    }
    None
}

/// Decode a raw `struct input_event` (x86_64 layout):
/// `u64 sec, u64 usec, u16 type, u16 code, i32 value`.
/// Returns the brightness delta for a press/repeat of the brightness keys.
fn key_delta(buf: &[u8; INPUT_EVENT_SIZE]) -> Option<i32> {
    let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
    let ev_code = u16::from_ne_bytes([buf[18], buf[19]]);
    let ev_value = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);

    // EV_KEY, value 1 (press) or 2 (repeat/hold)
    if ev_type != EV_KEY || (ev_value != 1 && ev_value != 2) {
        return None;
    }
    match ev_code {
        KEY_BRIGHTNESSDOWN => Some(-STEP),
        KEY_BRIGHTNESSUP => Some(STEP),
        _ => None,
    }
}

/// Apply a delta and clamp to the 0..=100 range.
fn apply_delta(old: i32, delta: i32) -> i32 {
    old.saturating_add(delta).clamp(0, 100)
}

/// Main evdev loop — blocks forever reading key events.
fn run_evdev_loop(bus: &str) -> Result<(), String> {
    let dev_path = find_keyd_device()
        .ok_or_else(|| "keyd virtual keyboard not found".to_string())?;

    let c_path = CString::new(dev_path.as_str()).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "open {}: {}",
            dev_path,
            std::io::Error::last_os_error()
        ));
    }

    // Initialize brightness from DDC
    match ddc::ddc_read_vcp(bus, 0x10) {
        Ok((cur, _max)) => {
            BRIGHTNESS.store(cur as i32, Ordering::Relaxed);
            eprintln!(
                "[brightness] evdev={}, ddc current={}%, step={}%",
                dev_path, cur, STEP
            );
        }
        Err(_) => {
            BRIGHTNESS.store(50, Ordering::Relaxed);
            eprintln!(
                "[brightness] evdev={}, ddc read failed, assuming 50%, step={}%",
                dev_path, STEP
            );
        }
    }

    let mut buf = [0u8; INPUT_EVENT_SIZE];
    let mut last_key: Option<std::time::Instant> = None;

    loop {
        let n = unsafe {
            libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, INPUT_EVENT_SIZE)
        };
        if n < INPUT_EVENT_SIZE as isize {
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                unsafe { libc::close(fd) };
                return Err(format!("read evdev: {}", err));
            }
            continue;
        }

        let delta = match key_delta(&buf) {
            Some(d) => d,
            None => continue,
        };

        // Resync the cached level when idle: brightness may have been changed
        // meanwhile by the tray, Home Assistant or the monitor's own OSD.
        if last_key.map_or(true, |t| t.elapsed() > RESYNC_AFTER) {
            if let Ok((cur, _)) = ddc::ddc_read_vcp(bus, 0x10) {
                BRIGHTNESS.store((cur as i32).clamp(0, 100), Ordering::Relaxed);
            }
        }
        last_key = Some(std::time::Instant::now());

        let old = BRIGHTNESS.load(Ordering::Relaxed);
        let new = apply_delta(old, delta);
        BRIGHTNESS.store(new, Ordering::Relaxed);

        // Direct blocking DDC write — this thread is already dedicated to evdev,
        // so blocking here is fine and avoids spawning unbounded threads per
        // keypress (M8). The bus lock serializes with other DDC callers.
        if let Err(e) = ddc::ddc_write_vcp(bus, 0x10, new as u16) {
            eprintln!("[brightness] DDC write failed: {}", e);
        }

        // Fire-and-forget KDE OSD notification
        let bri_str = new.to_string();
        thread::spawn(move || {
            // status() waits for the child; a dropped spawn() would leave a
            // zombie process per key press.
            let _ = std::process::Command::new("qdbus6")
                .args([
                    "org.kde.plasmashell",
                    "/org/kde/osdService",
                    "org.kde.osdService.brightnessChanged",
                    &bri_str,
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ty: u16, code: u16, value: i32) -> [u8; INPUT_EVENT_SIZE] {
        let mut b = [0u8; INPUT_EVENT_SIZE];
        b[16..18].copy_from_slice(&ty.to_ne_bytes());
        b[18..20].copy_from_slice(&code.to_ne_bytes());
        b[20..24].copy_from_slice(&value.to_ne_bytes());
        b
    }

    #[test]
    fn key_press_and_repeat_map_to_delta() {
        assert_eq!(key_delta(&ev(EV_KEY, KEY_BRIGHTNESSUP, 1)), Some(STEP));
        assert_eq!(key_delta(&ev(EV_KEY, KEY_BRIGHTNESSUP, 2)), Some(STEP));
        assert_eq!(key_delta(&ev(EV_KEY, KEY_BRIGHTNESSDOWN, 1)), Some(-STEP));
    }

    #[test]
    fn key_release_other_keys_and_other_types_ignored() {
        assert_eq!(key_delta(&ev(EV_KEY, KEY_BRIGHTNESSUP, 0)), None);
        assert_eq!(key_delta(&ev(EV_KEY, 30, 1)), None);
        assert_eq!(key_delta(&ev(0, KEY_BRIGHTNESSUP, 1)), None);
        assert_eq!(key_delta(&[0u8; INPUT_EVENT_SIZE]), None);
    }

    #[test]
    fn delta_is_clamped() {
        assert_eq!(apply_delta(98, STEP), 100);
        assert_eq!(apply_delta(2, -STEP), 0);
        assert_eq!(apply_delta(50, STEP), 55);
        assert_eq!(apply_delta(i32::MAX, STEP), 100);
    }

    #[test]
    fn circadian_curve_shape() {
        assert_eq!(circadian_for_hour(0.0), 30);
        assert_eq!(circadian_for_hour(5.99), 30);
        assert_eq!(circadian_for_hour(6.0), 30);
        assert_eq!(circadian_for_hour(7.5), 50);
        assert_eq!(circadian_for_hour(9.0), 70);
        assert_eq!(circadian_for_hour(16.99), 70);
        assert_eq!(circadian_for_hour(19.0), 50);
        assert_eq!(circadian_for_hour(21.0), 30);
        assert_eq!(circadian_for_hour(23.99), 30);
        let now = circadian_brightness();
        assert!((30..=70).contains(&now));
    }
}
