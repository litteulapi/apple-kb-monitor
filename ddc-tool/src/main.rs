//! ddc-tool — Direct I2C DDC/CI monitor control (Rust)
//!
//! Read/write VCP codes at native I2C speed. No ddcutil dependency.
//! Uses I2C_RDWR ioctl for proper DDC/CI read (write request + read reply).

use std::env;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::thread;
use std::time::Duration;

const DDC_ADDR: u16 = 0x37;
const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_RDWR: libc::c_ulong = 0x0707;

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

fn ddc_write_vcp(path: &str, vcp: u8, value: u16) -> Result<(), String> {
    use std::ffi::CString;
    let c_path = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(format!("open: {}", std::io::Error::last_os_error()));
    }

    if unsafe { libc::ioctl(fd, 0x0703, 0x37i32) } < 0 {
        unsafe { libc::close(fd); }
        return Err(format!("ioctl: {}", std::io::Error::last_os_error()));
    }

    let payload: [u8; 6] = [0x51, 0x84, 0x03, vcp, (value >> 8) as u8, (value & 0xFF) as u8];
    let mut chk: u8 = 0x6E;
    for b in &payload { chk ^= b; }
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

fn ddc_read_vcp(path: &str, vcp: u8) -> Result<(u16, u16, u8), String> {
    let file = OpenOptions::new().read(true).write(true).open(path)
        .map_err(|e| format!("{}: {}", path, e))?;
    let fd = file.as_raw_fd();

    // Write VCP Get Request via I2C_RDWR
    let payload: [u8; 4] = [0x51, 0x82, 0x01, vcp];
    let mut chk: u8 = 0x6E;
    for b in &payload { chk ^= b; }
    let mut write_buf = payload.to_vec();
    write_buf.push(chk);

    let mut w_msg = [I2cMsg { addr: DDC_ADDR, flags: 0, len: write_buf.len() as u16, buf: write_buf.as_mut_ptr() }];
    let w_data = I2cRdwrData { msgs: w_msg.as_mut_ptr(), nmsgs: 1 };
    if unsafe { libc::ioctl(fd, I2C_RDWR, &w_data as *const _) } < 0 {
        return Err("I2C write failed".into());
    }

    thread::sleep(Duration::from_millis(50));

    // Read response via I2C_RDWR
    let mut read_buf = [0u8; 12];
    let mut r_msg = [I2cMsg { addr: DDC_ADDR, flags: 1, len: 12, buf: read_buf.as_mut_ptr() }];
    let r_data = I2cRdwrData { msgs: r_msg.as_mut_ptr(), nmsgs: 1 };
    if unsafe { libc::ioctl(fd, I2C_RDWR, &r_data as *const _) } < 0 {
        return Err("I2C read failed".into());
    }

    // Response: [0x6E(src), len|0x80, opcode, result, vcp_code, vcp_type, max_hi, max_lo, cur_hi, cur_lo, chk]
    let offset: usize = if read_buf[0] == 0x6E { 1 } else { 0 };
    let opcode = read_buf[offset + 1];
    if opcode != 0x02 {
        return Err(format!("opcode 0x{:02X}", opcode));
    }
    let vcp_type = read_buf[offset + 4];
    let max_val = ((read_buf[offset + 5] as u16) << 8) | read_buf[offset + 6] as u16;
    let cur_val = ((read_buf[offset + 7] as u16) << 8) | read_buf[offset + 8] as u16;
    Ok((cur_val, max_val, vcp_type))
}

struct VcpInfo { code: u8, name: &'static str }

const KNOWN_VCPS: &[VcpInfo] = &[
    VcpInfo { code: 0x10, name: "brightness" },
    VcpInfo { code: 0x12, name: "contrast" },
    VcpInfo { code: 0x14, name: "color_preset" },
    VcpInfo { code: 0x15, name: "picture_mode" },
    VcpInfo { code: 0x16, name: "red_gain" },
    VcpInfo { code: 0x18, name: "green_gain" },
    VcpInfo { code: 0x1A, name: "blue_gain" },
    VcpInfo { code: 0x60, name: "input_source" },
    VcpInfo { code: 0x62, name: "volume" },
    VcpInfo { code: 0x8D, name: "audio_mute" },
    VcpInfo { code: 0xAC, name: "h_freq" },
    VcpInfo { code: 0xAE, name: "v_freq" },
    VcpInfo { code: 0xB6, name: "display_tech" },
    VcpInfo { code: 0xC0, name: "usage_hours" },
    VcpInfo { code: 0xC8, name: "controller" },
    VcpInfo { code: 0xC9, name: "firmware" },
    VcpInfo { code: 0xD6, name: "power_mode" },
    VcpInfo { code: 0xDF, name: "vcp_version" },
    VcpInfo { code: 0xF4, name: "lg_custom_1" },
    VcpInfo { code: 0xF5, name: "lg_custom_2" },
    VcpInfo { code: 0xF6, name: "lg_custom_3" },
    VcpInfo { code: 0xF7, name: "lg_custom_4" },
    VcpInfo { code: 0xF8, name: "lg_custom_5" },
    VcpInfo { code: 0xF9, name: "lg_custom_6" },
    VcpInfo { code: 0xFA, name: "lg_custom_7" },
    VcpInfo { code: 0xFE, name: "lg_custom_8" },
];

fn parse_vcp(s: &str) -> Option<u8> {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("ddc-tool v1.0 — Direct I2C DDC/CI");
        eprintln!("  ddc-tool read <bus> <vcp|all>");
        eprintln!("  ddc-tool write <bus> <vcp> <value>");
        eprintln!("  ddc-tool json <bus>");
        std::process::exit(1);
    }

    let cmd = &args[1];
    let path = if args[2].starts_with("/dev/") { args[2].clone() } else { format!("/dev/i2c-{}", args[2]) };

    match cmd.as_str() {
        "write" => {
            let vcp = parse_vcp(&args[3]).expect("bad vcp");
            let val: u16 = args[4].parse().expect("bad value");
            ddc_write_vcp(&path, vcp, val).unwrap_or_else(|e| { eprintln!("{}", e); std::process::exit(1); });
            println!("OK");
        }
        "read" => {
            if args[3] == "all" {
                for v in KNOWN_VCPS {
                    match ddc_read_vcp(&path, v.code) {
                        Ok((cur, max, _)) => println!("0x{:02X} {:20} {:5} {:5}", v.code, v.name, cur, max),
                        Err(_) => println!("0x{:02X} {:20} -     -", v.code, v.name),
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            } else {
                let vcp = parse_vcp(&args[3]).expect("bad vcp");
                match ddc_read_vcp(&path, vcp) {
                    Ok((cur, max, t)) => println!("{} {} {}", cur, max, t),
                    Err(e) => { eprintln!("{}", e); std::process::exit(1); }
                }
            }
        }
        "json" => {
            print!("{{");
            let mut first = true;
            for v in KNOWN_VCPS {
                if let Ok((cur, max, _)) = ddc_read_vcp(&path, v.code) {
                    if !first { print!(","); }
                    print!("\"{}\":{{\"current\":{},\"max\":{}}}", v.name, cur, max);
                    first = false;
                }
                thread::sleep(Duration::from_millis(50));
            }
            println!("}}");
        }
        _ => { eprintln!("unknown: {}", cmd); std::process::exit(1); }
    }
}
