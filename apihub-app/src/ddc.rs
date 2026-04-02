//! DDC/CI over raw I2C — no subprocess, no ddcutil.
//!
//! Uses libc::open + ioctl(I2C_SLAVE/I2C_RDWR) + libc::write/read directly.
//! Protocol matches the proven ddc-tool implementation.

use std::collections::HashMap;
use std::ffi::CString;
use std::thread;
use std::time::Duration;

const DDC_ADDR: u16 = 0x37;
const I2C_RDWR: libc::c_ulong = 0x0707;

/// Default I2C bus path for the monitor.
pub const DEFAULT_BUS: &str = "/dev/i2c-6";

// ── I2C kernel structs ──────────────────────────────────────────────────────

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

#[repr(C)]
struct I2cRdwrData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

// ── VCP metadata ────────────────────────────────────────────────────────────

pub struct VcpInfo {
    pub code: u8,
    pub name: &'static str,
}

/// The 30 essential VCPs we care about.
pub const ESSENTIAL_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0x10, name: "brightness" },
    VcpInfo { code: 0x12, name: "contrast" },
    VcpInfo { code: 0x14, name: "color_preset" },
    VcpInfo { code: 0x15, name: "picture_mode" },
    VcpInfo { code: 0x16, name: "red_gain" },
    VcpInfo { code: 0x18, name: "green_gain" },
    VcpInfo { code: 0x1A, name: "blue_gain" },
    VcpInfo { code: 0x60, name: "input_source" },
    VcpInfo { code: 0x62, name: "volume" },
    VcpInfo { code: 0x69, name: "color_temp_kelvin" },
    VcpInfo { code: 0x87, name: "sharpness" },
    VcpInfo { code: 0x8D, name: "audio_mute" },
    VcpInfo { code: 0xAC, name: "h_freq" },
    VcpInfo { code: 0xAE, name: "v_freq" },
    VcpInfo { code: 0xB6, name: "display_tech" },
    VcpInfo { code: 0xC0, name: "usage_hours" },
    VcpInfo { code: 0xC1, name: "backlight_pwm" },
    VcpInfo { code: 0xC9, name: "firmware" },
    VcpInfo { code: 0xCA, name: "osd_lock" },
    VcpInfo { code: 0xCC, name: "language" },
    VcpInfo { code: 0xD6, name: "power_mode" },
    VcpInfo { code: 0xD7, name: "split_mode" },
    VcpInfo { code: 0xDF, name: "vcp_version" },
    VcpInfo { code: 0xF5, name: "aspect_ratio" },
    VcpInfo { code: 0xF6, name: "smart_energy" },
    VcpInfo { code: 0xF7, name: "response_time" },
    VcpInfo { code: 0xF8, name: "freesync" },
    VcpInfo { code: 0xF9, name: "black_stabilizer" },
    VcpInfo { code: 0xFD, name: "power_led" },
    VcpInfo { code: 0xFE, name: "gamma" },
];

// ── Low-level I2C ───────────────────────────────────────────────────────────

/// Write a VCP value via I2C_SLAVE + raw write (proven path from ddc-tool).
pub fn ddc_write_vcp(path: &str, vcp: u8, value: u16) -> Result<(), String> {
    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(format!("open {}: {}", path, std::io::Error::last_os_error()));
    }

    if unsafe { libc::ioctl(fd, 0x0703, 0x37i32) } < 0 {
        unsafe { libc::close(fd); }
        return Err(format!("ioctl I2C_SLAVE: {}", std::io::Error::last_os_error()));
    }

    let payload: [u8; 6] = [
        0x51,
        0x84,
        0x03,
        vcp,
        (value >> 8) as u8,
        (value & 0xFF) as u8,
    ];
    let mut chk: u8 = 0x6E;
    for b in &payload {
        chk ^= b;
    }
    let mut msg = [0u8; 7];
    msg[..6].copy_from_slice(&payload);
    msg[6] = chk;

    let ret = unsafe { libc::write(fd, msg.as_ptr() as *const libc::c_void, 7) };
    unsafe { libc::close(fd); }

    if ret < 0 {
        return Err(format!("write: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Read a VCP value via I2C_RDWR ioctl (write request + read reply).
/// Returns (current, max).
pub fn ddc_read_vcp(path: &str, vcp: u8) -> Result<(u16, u16), String> {
    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(format!("open {}: {}", path, std::io::Error::last_os_error()));
    }

    // Build VCP Get Request
    let payload: [u8; 4] = [0x51, 0x82, 0x01, vcp];
    let mut chk: u8 = 0x6E;
    for b in &payload {
        chk ^= b;
    }
    let mut write_buf = [0u8; 5];
    write_buf[..4].copy_from_slice(&payload);
    write_buf[4] = chk;

    let mut w_msg = I2cMsg {
        addr: DDC_ADDR,
        flags: 0,
        len: write_buf.len() as u16,
        buf: write_buf.as_mut_ptr(),
    };
    let mut w_data = I2cRdwrData {
        msgs: &mut w_msg as *mut _,
        nmsgs: 1,
    };
    if unsafe { libc::ioctl(fd, I2C_RDWR, &mut w_data as *mut _) } < 0 {
        unsafe { libc::close(fd); }
        return Err(format!("I2C write 0x{:02X}: {}", vcp, std::io::Error::last_os_error()));
    }

    thread::sleep(Duration::from_millis(60));

    // Read response
    let mut read_buf = [0u8; 12];
    let mut r_msg = I2cMsg {
        addr: DDC_ADDR,
        flags: 1,
        len: 12,
        buf: read_buf.as_mut_ptr(),
    };
    let mut r_data = I2cRdwrData {
        msgs: &mut r_msg as *mut _,
        nmsgs: 1,
    };
    if unsafe { libc::ioctl(fd, I2C_RDWR, &mut r_data as *mut _) } < 0 {
        unsafe { libc::close(fd); }
        return Err(format!("I2C read 0x{:02X}: {}", vcp, std::io::Error::last_os_error()));
    }
    unsafe { libc::close(fd); }

    // Parse DDC/CI response
    let offset: usize = if read_buf[0] == 0x6E { 1 } else { 0 };
    let opcode = read_buf[offset + 1];
    if opcode != 0x02 {
        return Err(format!("VCP 0x{:02X}: bad opcode 0x{:02X}", vcp, opcode));
    }
    let max_val = ((read_buf[offset + 5] as u16) << 8) | read_buf[offset + 6] as u16;
    let cur_val = ((read_buf[offset + 7] as u16) << 8) | read_buf[offset + 8] as u16;
    Ok((cur_val, max_val))
}

/// Read all 30 essential VCPs. Returns name -> (current, max).
/// Tolerates individual read failures (skips them).
pub fn read_all_essential(path: &str) -> HashMap<String, (u16, u16)> {
    let mut map = HashMap::new();
    for v in ESSENTIAL_VCPS {
        if let Ok((cur, max)) = ddc_read_vcp(path, v.code) {
            map.insert(v.name.to_string(), (cur, max));
        }
        thread::sleep(Duration::from_millis(50));
    }
    map
}
