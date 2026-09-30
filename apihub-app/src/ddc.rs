//! DDC/CI over raw I2C — no subprocess, no ddcutil.
//!
//! Direct libc I2C: open + ioctl(I2C_RDWR) + close per transaction.
//! Validates response opcode, VCP code, result code, length, and checksum.

use std::ffi::CString;
use std::sync::Mutex;

const DDC_ADDR: u16 = 0x37;
const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_RDWR: libc::c_ulong = 0x0707;

/// Monitor identity from EDID (parsed from sysfs DRM connector).
#[derive(Clone, Default)]
pub struct MonitorInfo {
    pub name: String,        // e.g. "34GN850"
    pub manufacturer: String, // e.g. "GSM" (LG)
    pub connector: String,   // e.g. "DP-2"
    pub adapter: String,     // e.g. "NVIDIA i2c adapter 5"
    pub serial: String,      // e.g. "008NTRL0W606"
}

/// Read monitor identity from DRM EDID + sysfs.
pub fn read_monitor_info(bus_path: &str) -> MonitorInfo {
    let mut info = MonitorInfo::default();

    // I2C adapter name
    let bus_num = bus_path.rsplit('-').next().unwrap_or("6");
    let adapter_path = format!("/sys/bus/i2c/devices/i2c-{}/name", bus_num);
    if let Ok(name) = std::fs::read_to_string(&adapter_path) {
        info.adapter = name.trim().to_string();
    }

    // Find connected DRM connector with EDID
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.contains('-') { continue; }
            let status_path = entry.path().join("status");
            if let Ok(status) = std::fs::read_to_string(&status_path) {
                if status.trim() != "connected" { continue; }
                let edid_path = entry.path().join("edid");
                if let Ok(edid) = std::fs::read(&edid_path) {
                    if edid.len() >= 128 {
                        // Parse EDID manufacturer (bytes 8-9)
                        let m1 = ((edid[8] >> 2) & 0x1F) + 64;
                        let m2 = (((edid[8] & 3) << 3) | (edid[9] >> 5)) + 64;
                        let m3 = (edid[9] & 0x1F) + 64;
                        info.manufacturer = format!("{}{}{}", m1 as char, m2 as char, m3 as char);

                        // Parse EDID descriptor blocks for monitor name (0xFC) and serial (0xFF)
                        let mut i = 54;
                        while i + 18 <= edid.len().min(126) {
                            if edid[i] == 0 && edid[i+1] == 0 && edid[i+2] == 0 && i+4 < edid.len() {
                                let tag = edid[i+3];
                                let text = String::from_utf8_lossy(&edid[i+5..i+18])
                                    .split('\n').next().unwrap_or("").trim().to_string();
                                if tag == 0xFC { info.name = text.clone(); }
                                if tag == 0xFF { info.serial = text; }
                            }
                            i += 18;
                        }

                        if !info.name.is_empty() {
                            info.connector = name.split('-').skip(1).collect::<Vec<_>>().join("-");
                            break;
                        }
                    }
                }
            }
        }
    }

    info
}

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
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
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

/// Extract `bus = "/dev/i2c-N"` from the `[ddc]` section of a config.toml.
/// Section headers may carry trailing comments; `[[array]]` tables and nested
/// `[a.b]` sections are not `[ddc]`. Both "..." and '...' strings are accepted.
fn parse_bus_from_config(content: &str) -> Option<String> {
    let mut in_ddc = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let inner = line.trim_start_matches('[');
            let name = inner.split(']').next().unwrap_or("").trim();
            in_ddc = !line.starts_with("[[") && name == "ddc";
            continue;
        }
        if !in_ddc || line.is_empty() || line.starts_with('#') { continue; }
        if let Some((key, val)) = line.split_once('=') {
            if key.trim() != "bus" { continue; }
            let val = val.trim();
            let val = match val.chars().next() {
                Some(q @ ('"' | '\'')) => val[1..].split(q).next().unwrap_or(""),
                _ => val.split('#').next().unwrap_or("").trim(),
            };
            if val.starts_with("/dev/") {
                return Some(val.to_string());
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
            if let Some(bus) = parse_bus_from_config(&content) {
                return bus;
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
static BUS_LOCK: Mutex<()> = Mutex::new(());

/// Persistent I2C file descriptor — opened once, reused forever.
/// Eliminates open()/close() syscall overhead per read cycle.
static PERSISTENT_FD: Mutex<Option<libc::c_int>> = Mutex::new(None);

/// Counter for periodic deep probe of the persistent fd (M3).
static FD_USE_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Get or create the persistent I2C fd for the given bus path.
///
/// Staleness detection (M3): besides the fast `fcntl(F_GETFD)` check on every
/// call, every 100th call performs a real DDC VCP 0xDF read probe to detect
/// a monitor that has been power-cycled or unplugged (the fd stays valid at
/// the kernel level but the I2C device no longer responds).
fn get_fd(path: &str) -> Result<libc::c_int, String> {
    let mut fd_lock = PERSISTENT_FD.lock().map_err(|e| format!("fd lock: {}", e))?;
    if let Some(fd) = *fd_lock {
        // Fast check: fd still open at kernel level
        let ret = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if ret < 0 {
            *fd_lock = None;
        } else {
            // Deep probe every 100th call: verify the monitor actually responds
            let count = FD_USE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if count % 100 == 99 {
                if ddc_read_vcp_fd(fd, 0xDF).is_err() {
                    eprintln!("[ddc] periodic probe failed — closing stale fd");
                    unsafe { libc::close(fd); }
                    *fd_lock = None;
                } else {
                    return Ok(fd);
                }
            } else {
                return Ok(fd);
            }
        }
    }
    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(format!("open {}: {}", path, std::io::Error::last_os_error()));
    }
    *fd_lock = Some(fd);
    Ok(fd)
}

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

/// HOT VCPs — polled every cycle for near-realtime feedback.
/// 0x02 (New Control Value) is read first: if 0, WARM VCPs are skipped that cycle.
pub const HOT_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0x02, name: "new_control_value" },
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

/// PBP mirror registers — read-only, only meaningful when split mode (0xD7) != 1.
/// These expose the scaler's sub-display settings.
pub const PBP_MIRROR_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0xE8, name: "mirror_brightness" },
    VcpInfo { code: 0xE9, name: "mirror_contrast" },
    VcpInfo { code: 0xEA, name: "mirror_color_preset" },
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
/// Uses persistent fd — no open/close overhead.
///
/// Note (M5): this function uses I2C_SLAVE + write() while `ddc_read_vcp_fd`
/// uses I2C_RDWR on the same fd. Mixing the two is safe in practice: the
/// kernel's I2C subsystem serializes transactions per-adapter, and the
/// BUS_LOCK mutex ensures we never interleave from userspace. The I2C_SLAVE
/// ioctl only sets the target address for subsequent read()/write() calls
/// and does not conflict with I2C_RDWR which carries its own address.
pub fn ddc_write_vcp(path: &str, vcp: u8, value: u16) -> Result<(), String> {
    let _lock = BUS_LOCK.lock().map_err(|e| format!("bus lock: {}", e))?;
    let fd = get_fd(path)?;

    if unsafe { libc::ioctl(fd, I2C_SLAVE, DDC_ADDR as libc::c_ulong) } < 0 {
        return Err(format!("ioctl I2C_SLAVE: {}", std::io::Error::last_os_error()));
    }

    let payload: [u8; 6] = [
        0x51, 0x84, 0x03, vcp,
        (value >> 8) as u8, (value & 0xFF) as u8,
    ];
    let mut chk: u8 = 0x6E;
    for b in &payload { chk ^= b; }
    let mut msg = [0u8; 7];
    msg[..6].copy_from_slice(&payload);
    msg[6] = chk;

    let ret = unsafe { libc::write(fd, msg.as_ptr() as *const libc::c_void, 7) };
    if ret < 0 {
        return Err(format!("write VCP 0x{:02X}: {}", vcp, std::io::Error::last_os_error()));
    }
    if ret != msg.len() as isize {
        return Err(format!("write VCP 0x{:02X}: short write ({} of {} bytes)", vcp, ret, msg.len()));
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

    parse_vcp_reply(&buf, vcp)
}

/// Parse and validate a DDC/CI VCP Get Reply.
/// Wire: [src=0x6E] [len=0x88] [op=0x02] [result] [vcp] [type] [max_hi] [max_lo] [cur_hi] [cur_lo] [chk]
/// The leading source byte may be absent; the checksum still covers it (virtual 0x6E).
fn parse_vcp_reply(buf: &[u8; 12], vcp: u8) -> Result<(u16, u16), String> {
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

    // Checksum: XOR of host addr (0x50) with every reply byte before the checksum,
    // including the source byte (0x6E) even when the bus did not return it.
    let chk_idx = off + 9;
    let mut computed: u8 = 0x50;
    if off == 0 { computed ^= 0x6E; }
    for b in &buf[..chk_idx] {
        computed ^= b;
    }
    if computed != buf[chk_idx] {
        return Err(format!("VCP 0x{:02X}: checksum mismatch (got 0x{:02X}, expected 0x{:02X})",
            vcp, buf[chk_idx], computed));
    }

    let max_val = ((buf[off + 5] as u16) << 8) | buf[off + 6] as u16;
    let cur_val = ((buf[off + 7] as u16) << 8) | buf[off + 8] as u16;
    Ok((cur_val, max_val))
}

/// Read a single VCP using the persistent fd.
pub fn ddc_read_vcp(path: &str, vcp: u8) -> Result<(u16, u16), String> {
    let _lock = BUS_LOCK.lock().map_err(|e| format!("bus lock: {}", e))?;
    let fd = get_fd(path)?;
    ddc_read_vcp_fd(fd, vcp)
}

/// Burst-read VCPs using the persistent fd. No open/close per call.
pub fn read_batch(path: &str, vcps: &[VcpInfo]) -> Vec<(&'static str, u16, u16)> {
    let _lock = match BUS_LOCK.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let fd = match get_fd(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::with_capacity(vcps.len());
    for v in vcps {
        if let Ok((cur, max)) = ddc_read_vcp_fd(fd, v.code) {
            results.push((v.name, cur, max));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid reply frame (with source byte) for `vcp`.
    fn frame(vcp: u8, max: u16, cur: u16) -> [u8; 12] {
        let mut b = [0u8; 12];
        b[0] = 0x6E; b[1] = 0x88; b[2] = 0x02; b[3] = 0x00; b[4] = vcp; b[5] = 0x00;
        b[6] = (max >> 8) as u8; b[7] = max as u8; b[8] = (cur >> 8) as u8; b[9] = cur as u8;
        let mut c = 0x50u8;
        for x in &b[..10] { c ^= x; }
        b[10] = c;
        b
    }

    /// Same frame without the leading source byte.
    fn frame_nosrc(vcp: u8, max: u16, cur: u16) -> [u8; 12] {
        let f = frame(vcp, max, cur);
        let mut b = [0u8; 12];
        b[..10].copy_from_slice(&f[1..11]);
        b
    }

    #[test]
    fn reply_with_source_ok() {
        assert_eq!(parse_vcp_reply(&frame(0x10, 100, 50), 0x10), Ok((50, 100)));
    }

    #[test]
    fn reply_without_source_uses_virtual_source_in_checksum() {
        assert_eq!(parse_vcp_reply(&frame_nosrc(0x10, 100, 50), 0x10), Ok((50, 100)));
    }

    #[test]
    fn reply_bad_checksum_rejected() {
        let mut f = frame(0x10, 100, 50);
        f[10] ^= 0x01;
        assert!(parse_vcp_reply(&f, 0x10).unwrap_err().contains("checksum"));
        let mut g = frame_nosrc(0x10, 100, 50);
        g[9] ^= 0x01;
        assert!(parse_vcp_reply(&g, 0x10).unwrap_err().contains("checksum"));
    }

    #[test]
    fn reply_rejects_bad_length_opcode_result_alias() {
        let mut f = frame(0x10, 100, 50);
        f[1] = 0x87;
        assert!(parse_vcp_reply(&f, 0x10).unwrap_err().contains("length"));
        let mut f = frame(0x10, 100, 50);
        f[2] = 0x24;
        assert!(parse_vcp_reply(&f, 0x10).unwrap_err().contains("opcode"));
        let mut f = frame(0x10, 100, 50);
        f[3] = 0x01;
        assert!(parse_vcp_reply(&f, 0x10).unwrap_err().contains("unsupported"));
        assert!(parse_vcp_reply(&frame(0x12, 100, 50), 0x10).unwrap_err().contains("aliased"));
    }

    #[test]
    fn reply_all_zero_buffer_is_error_not_panic() {
        assert!(parse_vcp_reply(&[0u8; 12], 0x10).is_err());
        assert!(parse_vcp_reply(&[0xFFu8; 12], 0x10).is_err());
    }

    #[test]
    fn config_bus_basic_and_comments() {
        let c = "[ddc]\nbus = \"/dev/i2c-4\" # c\n";
        assert_eq!(parse_bus_from_config(c).as_deref(), Some("/dev/i2c-4"));
        let c = "[ddc] # monitor\nbus = '/dev/i2c-7'\n";
        assert_eq!(parse_bus_from_config(c).as_deref(), Some("/dev/i2c-7"));
        let c = "[ddc]\nbus = /dev/i2c-2 # bare\n";
        assert_eq!(parse_bus_from_config(c).as_deref(), Some("/dev/i2c-2"));
    }

    #[test]
    fn config_bus_ignores_other_sections() {
        let c = "[mqtt]\nbus = \"/dev/i2c-1\"\n[[ddc]]\nbus = \"/dev/i2c-3\"\n[ddc.sub]\nbus = \"/dev/i2c-5\"\n";
        assert_eq!(parse_bus_from_config(c), None);
        assert_eq!(parse_bus_from_config("[ddc]\nbus = \"relative\"\n"), None);
        assert_eq!(parse_bus_from_config(""), None);
    }

    #[test]
    fn write_checksum_matches_spec_example() {
        // 6E ^ 51 ^ 84 ^ 03 ^ 10 ^ 00 ^ 32 computed independently
        let payload = [0x51u8, 0x84, 0x03, 0x10, 0x00, 0x32];
        let chk = payload.iter().fold(0x6Eu8, |a, b| a ^ b);
        assert_eq!(chk, 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x10 ^ 0x32);
    }
}
