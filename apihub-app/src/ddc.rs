//! DDC/CI over raw I2C — no subprocess, no ddcutil.
//!
//! Direct libc I2C: open + ioctl(I2C_RDWR) + close per transaction.
//! Validates response opcode, VCP code, result code, length, and checksum.

use std::ffi::CString;
use std::sync::Mutex;

const DDC_ADDR: u16 = 0x37;
const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_RDWR: libc::c_ulong = 0x0707;

/// Auto-detect I2C bus by probing for a DDC-capable display.
/// Tests each /dev/i2c-* for a valid DDC/CI response to VCP 0xDF (version).
pub fn detect_bus() -> Option<String> {
    if let Ok(rd) = std::fs::read_dir("/dev") {
        let mut buses: Vec<String> = rd.filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("i2c-"))
            .map(|e| format!("/dev/{}", e.file_name().to_string_lossy()))
            .collect();
        buses.sort();
        for bus in &buses {
            let c_path = match CString::new(bus.as_str()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
            if fd < 0 { continue; }
            let result = ddc_read_vcp_fd(fd, 0xDF); // VCP version
            unsafe { libc::close(fd); }
            if result.is_ok() {
                eprintln!("[ddc] auto-detected monitor on {}", bus);
                return Some(bus.clone());
            }
        }
    }
    None
}

/// Default I2C bus path for the monitor.
/// Priority: config.toml > auto-detect > fallback /dev/i2c-6
pub fn default_bus() -> String {
    let paths = [
        dirs::config_dir().map(|d| d.join("apple-kb-monitor/config.toml")),
        Some(std::path::PathBuf::from("/etc/apple-kb-monitor/config.toml")),
    ];
    for path in paths.into_iter().flatten() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let mut in_ddc = false;
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_ddc = line.trim_matches(|c| c == '[' || c == ']').trim() == "ddc";
                    continue;
                }
                if !in_ddc { continue; }
                if line.is_empty() || line.starts_with('#') { continue; }
                if let Some((key, val)) = line.split_once('=') {
                    if key.trim() == "bus" {
                        let val = val.trim();
                        let val = if val.starts_with('"') {
                            val.trim_start_matches('"')
                                .splitn(2, '"').next().unwrap_or("")
                        } else {
                            val.split('#').next().unwrap_or("").trim()
                        };
                        if val.starts_with("/dev/") {
                            return val.to_string();
                        }
                    }
                }
            }
        }
    }
    // Auto-detect: probe all I2C buses for DDC
    if let Some(bus) = detect_bus() {
        return bus;
    }
    "/dev/i2c-6".to_string()
}

/// Global I2C bus lock — serializes all DDC transactions on the same bus.
/// Prevents concurrent reads/writes from corrupting the I2C pipeline.
static BUS_LOCK: Mutex<()> = Mutex::new(());

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

/// HOT VCPs — the 3 values most likely to change externally (~210ms burst).
/// Polled every cycle for near-realtime feedback.
pub const HOT_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0x10, name: "brightness" },
    VcpInfo { code: 0x62, name: "volume" },
    VcpInfo { code: 0xC1, name: "backlight_pwm" },
];

/// WARM VCPs — user-adjustable controls (~420ms burst).
/// Polled every 4th cycle (~3s).
pub const WARM_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0x12, name: "contrast" },
    VcpInfo { code: 0x16, name: "red_gain" },
    VcpInfo { code: 0x18, name: "green_gain" },
    VcpInfo { code: 0x1A, name: "blue_gain" },
    VcpInfo { code: 0x6C, name: "black_level_red" },
    VcpInfo { code: 0x6E, name: "black_level_green" },
    VcpInfo { code: 0x70, name: "black_level_blue" },
    VcpInfo { code: 0x87, name: "sharpness" },
    VcpInfo { code: 0xF9, name: "black_stabilizer" },
];

/// COLD VCPs — settings/info that rarely change (~1.5s burst).
/// Polled every 30th cycle (~22s).
pub const COLD_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0x14, name: "color_preset" },
    VcpInfo { code: 0x15, name: "picture_mode" },
    VcpInfo { code: 0x60, name: "input_source" },
    VcpInfo { code: 0x69, name: "color_temp_kelvin" },
    VcpInfo { code: 0x72, name: "gamma_curve" },
    VcpInfo { code: 0x8D, name: "audio_mute" },
    VcpInfo { code: 0xAC, name: "h_freq" },
    VcpInfo { code: 0xAE, name: "v_freq" },
    VcpInfo { code: 0xB6, name: "display_tech" },
    VcpInfo { code: 0xC0, name: "usage_hours" },
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
    VcpInfo { code: 0xFD, name: "power_led" },
    VcpInfo { code: 0xFE, name: "gamma" },
];

/// All 31 VCPs (for optimistic update lookup and diagnostics).
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
    VcpInfo { code: 0x6C, name: "black_level_red" },
    VcpInfo { code: 0x6E, name: "black_level_green" },
    VcpInfo { code: 0x70, name: "black_level_blue" },
    VcpInfo { code: 0x72, name: "gamma_curve" },
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

/// Write a VCP value via I2C_SLAVE + raw write.
/// NVIDIA I2C adapters require this path (I2C_RDWR fails for writes).
pub fn ddc_write_vcp(path: &str, vcp: u8, value: u16) -> Result<(), String> {
    let _lock = BUS_LOCK.lock().map_err(|e| format!("bus lock: {}", e))?;

    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(format!("open {}: {}", path, std::io::Error::last_os_error()));
    }

    if unsafe { libc::ioctl(fd, I2C_SLAVE, DDC_ADDR as libc::c_ulong) } < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd); }
        return Err(format!("ioctl I2C_SLAVE: {}", err));
    }

    // DDC/CI VCP Set: [0x51] [0x84] [0x03] [vcp] [val_hi] [val_lo] [chk]
    let payload: [u8; 6] = [
        0x51, 0x84, 0x03, vcp,
        (value >> 8) as u8, (value & 0xFF) as u8,
    ];
    let mut chk: u8 = 0x6E; // seed = destination address (0x37 << 1)
    for b in &payload { chk ^= b; }
    let mut msg = [0u8; 7];
    msg[..6].copy_from_slice(&payload);
    msg[6] = chk;

    let ret = unsafe { libc::write(fd, msg.as_ptr() as *const libc::c_void, 7) };
    let err = std::io::Error::last_os_error(); // capture BEFORE close
    unsafe { libc::close(fd); }

    if ret < 0 {
        return Err(format!("write VCP 0x{:02X}: {}", vcp, err));
    }
    Ok(())
}

/// Combined write+read in a single I2C_RDWR ioctl (2 messages).
/// The kernel handles timing between write and read — no userspace sleep needed.
/// 100% reliable on NVIDIA I2C adapters (kernel 6.x+).
fn ddc_read_vcp_fd(fd: libc::c_int, vcp: u8) -> Result<(u16, u16), String> {
    // DDC/CI VCP Get request
    let payload: [u8; 4] = [0x51, 0x82, 0x01, vcp];
    let mut chk: u8 = 0x6E;
    for b in &payload { chk ^= b; }
    let mut write_buf = [0u8; 5];
    write_buf[..4].copy_from_slice(&payload);
    write_buf[4] = chk;

    let mut buf = [0u8; 12];

    // 2 messages in a single ioctl: write request + read response
    let mut msgs = [
        I2cMsg { addr: DDC_ADDR, flags: 0, len: 5, buf: write_buf.as_mut_ptr() },
        I2cMsg { addr: DDC_ADDR, flags: 1, len: 12, buf: buf.as_mut_ptr() },
    ];
    let mut data = I2cRdwrData { msgs: msgs.as_mut_ptr(), nmsgs: 2 };

    if unsafe { libc::ioctl(fd, I2C_RDWR, &mut data as *mut _) } < 0 {
        return Err(format!("I2C combined 0x{:02X}: {}", vcp, std::io::Error::last_os_error()));
    }

    // Parse DDC/CI VCP Get Reply
    // Wire: [src=0x6E] [len=0x88] [op=0x02] [result] [vcp] [type] [max_hi] [max_lo] [cur_hi] [cur_lo] [chk]
    let off: usize = if buf[0] == 0x6E { 1 } else { 0 };

    // Validate length byte (0x88 = 0x80 | 8 data bytes)
    if buf[off] != 0x88 {
        return Err(format!("VCP 0x{:02X}: bad length 0x{:02X}", vcp, buf[off]));
    }

    // Validate opcode
    let opcode = buf[off + 1];
    if opcode != 0x02 {
        return Err(format!("VCP 0x{:02X}: bad opcode 0x{:02X}", vcp, opcode));
    }

    // Validate result code (0x00 = success, 0x01 = unsupported)
    let result = buf[off + 2];
    if result != 0x00 {
        return Err(format!("VCP 0x{:02X}: unsupported (result=0x{:02X})", vcp, result));
    }

    // Validate VCP code in response (detect I2C pipeline aliasing)
    let resp_vcp = buf[off + 3];
    if resp_vcp != vcp {
        return Err(format!("VCP 0x{:02X}: aliased (got 0x{:02X})", vcp, resp_vcp));
    }

    // Validate checksum: XOR of host addr (0x50) with ALL reply bytes including source (0x6E)
    // Formula: chk = 0x50 ^ buf[0] ^ buf[1] ^ ... ^ buf[9], must equal buf[10]
    let chk_idx = off + 9;
    if chk_idx < buf.len() {
        let mut computed: u8 = 0x50;
        for i in 0..=chk_idx - 1 {
            computed ^= buf[i];
        }
        if computed != buf[chk_idx] {
            return Err(format!("VCP 0x{:02X}: checksum mismatch (got 0x{:02X}, expected 0x{:02X})",
                vcp, buf[chk_idx], computed));
        }
    }

    let max_val = ((buf[off + 5] as u16) << 8) | buf[off + 6] as u16;
    let cur_val = ((buf[off + 7] as u16) << 8) | buf[off + 8] as u16;
    Ok((cur_val, max_val))
}

/// Read a single VCP value with validation and auto-retry.
/// Used by diagnostics and single-VCP reads. For bulk reads, use read_burst().
#[allow(dead_code)]
pub fn ddc_read_vcp(path: &str, vcp: u8) -> Result<(u16, u16), String> {
    let _lock = BUS_LOCK.lock().map_err(|e| format!("bus lock: {}", e))?;

    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(format!("open {}: {}", path, std::io::Error::last_os_error()));
    }

    let result = ddc_read_vcp_fd(fd, vcp);
    unsafe { libc::close(fd); }
    result
}

/// Burst-read VCPs: single fd, combined I2C_RDWR per VCP (no sleep between reads).
/// 100% reliable with combined 2-message ioctl — kernel handles I2C timing.
pub fn read_batch(path: &str, vcps: &[VcpInfo]) -> Vec<(&'static str, u16, u16)> {
    let _lock = match BUS_LOCK.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let c_path = match CString::new(path) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 { return Vec::new(); }

    let mut results = Vec::with_capacity(vcps.len());
    for v in vcps {
        if let Ok((cur, max)) = ddc_read_vcp_fd(fd, v.code) {
            results.push((v.name, cur, max));
        }
    }
    unsafe { libc::close(fd); }
    results
}
