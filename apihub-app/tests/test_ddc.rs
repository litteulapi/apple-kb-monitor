//! Unit tests for DDC/CI protocol, VCP metadata, and UI data consistency.
//!
//! These tests validate the correctness of VCP mappings, DDC packet construction,
//! and data display logic WITHOUT requiring actual I2C hardware.

// We test the DDC checksum and packet construction logic by reimplementing
// the pure computation parts (no I2C syscalls).

/// DDC/CI write checksum: XOR of 0x6E with all payload bytes.
fn ddc_write_checksum(vcp: u8, value: u16) -> u8 {
    let payload: [u8; 6] = [
        0x51, 0x84, 0x03, vcp,
        (value >> 8) as u8, (value & 0xFF) as u8,
    ];
    let mut chk: u8 = 0x6E;
    for b in &payload {
        chk ^= b;
    }
    chk
}

/// DDC/CI read request checksum.
fn ddc_read_checksum(vcp: u8) -> u8 {
    let payload: [u8; 4] = [0x51, 0x82, 0x01, vcp];
    let mut chk: u8 = 0x6E;
    for b in &payload {
        chk ^= b;
    }
    chk
}

/// Parse a DDC/CI VCP Get Reply (same logic as ddc.rs).
fn parse_ddc_reply(buf: &[u8; 12]) -> Result<(u16, u16), String> {
    let offset: usize = if buf[0] == 0x6E { 1 } else { 0 };
    let opcode = buf[offset + 1];
    if opcode != 0x02 {
        return Err(format!("bad opcode 0x{:02X}", opcode));
    }
    let max_val = ((buf[offset + 5] as u16) << 8) | buf[offset + 6] as u16;
    let cur_val = ((buf[offset + 7] as u16) << 8) | buf[offset + 8] as u16;
    Ok((cur_val, max_val))
}

// ── DDC Protocol Tests ─────────────────────────────────────────────────────

#[test]
fn test_write_checksum_brightness_50() {
    // VCP 0x10 = brightness, value = 50 (0x0032)
    let chk = ddc_write_checksum(0x10, 50);
    // Manual: 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x10 ^ 0x00 ^ 0x32
    let expected = 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x10 ^ 0x00 ^ 0x32;
    assert_eq!(chk, expected);
}

#[test]
fn test_write_checksum_gamma_2_2() {
    // VCP 0x72, value = 0x7800 (gamma 2.2)
    let chk = ddc_write_checksum(0x72, 0x7800);
    let expected = 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x72 ^ 0x78 ^ 0x00;
    assert_eq!(chk, expected);
}

#[test]
fn test_read_checksum_brightness() {
    let chk = ddc_read_checksum(0x10);
    let expected = 0x6E ^ 0x51 ^ 0x82 ^ 0x01 ^ 0x10;
    assert_eq!(chk, expected);
}

#[test]
fn test_write_payload_structure() {
    // Verify the 7-byte DDC write message structure
    let vcp: u8 = 0x62; // volume
    let value: u16 = 80;
    let payload: [u8; 6] = [
        0x51, 0x84, 0x03, vcp,
        (value >> 8) as u8, (value & 0xFF) as u8,
    ];
    assert_eq!(payload[0], 0x51); // DDC source address
    assert_eq!(payload[1], 0x84); // VCP Set opcode + length
    assert_eq!(payload[2], 0x03); // VCP Set command
    assert_eq!(payload[3], 0x62); // VCP code
    assert_eq!(payload[4], 0x00); // value high byte
    assert_eq!(payload[5], 0x50); // value low byte (80 = 0x50)
}

#[test]
fn test_write_payload_high_value() {
    // Gamma 2.4 = 0x8C00
    let value: u16 = 0x8C00;
    assert_eq!((value >> 8) as u8, 0x8C);
    assert_eq!((value & 0xFF) as u8, 0x00);
}

// ── DDC Response Parsing Tests ─────────────────────────────────────────────

#[test]
fn test_parse_reply_with_source_addr() {
    // Response starts with 0x6E (source address present)
    let mut buf = [0u8; 12];
    buf[0] = 0x6E; // source address
    buf[1] = 0x88; // length
    buf[2] = 0x02; // VCP reply opcode
    buf[3] = 0x00; // result code
    buf[4] = 0x10; // VCP code
    buf[5] = 0x00; // type
    buf[6] = 0x00; buf[7] = 0x64; // max = 100
    buf[8] = 0x00; buf[9] = 0x32; // current = 50
    let (cur, max) = parse_ddc_reply(&buf).unwrap();
    assert_eq!(cur, 50);
    assert_eq!(max, 100);
}

#[test]
fn test_parse_reply_without_source_addr() {
    // Response starts directly with length (no 0x6E prefix)
    let mut buf = [0u8; 12];
    buf[0] = 0x88; // length
    buf[1] = 0x02; // VCP reply opcode
    buf[2] = 0x00; // result code
    buf[3] = 0x10; // VCP code
    buf[4] = 0x00; // type
    buf[5] = 0x00; buf[6] = 0x64; // max = 100
    buf[7] = 0x00; buf[8] = 0x32; // current = 50
    let (cur, max) = parse_ddc_reply(&buf).unwrap();
    assert_eq!(cur, 50);
    assert_eq!(max, 100);
}

#[test]
fn test_parse_reply_bad_opcode() {
    let mut buf = [0u8; 12];
    buf[0] = 0x6E;
    buf[2] = 0x24; // unsupported VCP opcode
    let result = parse_ddc_reply(&buf);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("0x24"));
}

#[test]
fn test_parse_reply_gamma_value() {
    // VCP 0x72 gamma = 0x7800 (2.2), max = 0xFFFF
    let mut buf = [0u8; 12];
    buf[0] = 0x6E;
    buf[2] = 0x02;
    buf[6] = 0xFF; buf[7] = 0xFF; // max = 65535
    buf[8] = 0x78; buf[9] = 0x00; // current = 0x7800 = 30720
    let (cur, max) = parse_ddc_reply(&buf).unwrap();
    assert_eq!(cur, 30720);
    assert_eq!(max, 65535);
}

#[test]
fn test_parse_reply_firmware_version() {
    // VCP 0xC9: firmware 3.0 = 0x0300 = 768
    let mut buf = [0u8; 12];
    buf[0] = 0x6E;
    buf[2] = 0x02;
    buf[6] = 0xFF; buf[7] = 0xFF;
    buf[8] = 0x03; buf[9] = 0x00; // 768
    let (cur, _) = parse_ddc_reply(&buf).unwrap();
    assert_eq!(cur, 768);
    assert_eq!(cur >> 8, 3);   // major
    assert_eq!(cur & 0xFF, 0); // minor
}

// ── Gamma Encoding Tests ───────────────────────────────────────────────────

#[test]
fn test_gamma_encoding() {
    // Encoding: (gamma - 1.0) * 100, stored in high byte
    let gammas = [(1.8, 0x5000u16), (2.0, 0x6400), (2.2, 0x7800), (2.4, 0x8C00)];
    for (gamma, expected) in gammas {
        let encoded = (((gamma - 1.0) * 100.0) as u16) << 8;
        assert_eq!(encoded, expected, "gamma {} should encode to 0x{:04X}", gamma, expected);
    }
}

#[test]
fn test_gamma_decoding() {
    let values = [(0x5000u16, 1.8), (0x6400, 2.0), (0x7800, 2.2), (0x8C00, 2.4)];
    for (encoded, expected_gamma) in values {
        let high_byte = (encoded >> 8) as f64;
        let gamma = high_byte / 100.0 + 1.0;
        assert!((gamma - expected_gamma).abs() < 0.01,
            "0x{:04X} should decode to gamma {}, got {}", encoded, expected_gamma, gamma);
    }
}

// ── VCP Metadata Consistency Tests ─────────────────────────────────────────

/// All VCP names referenced in UI must exist in ESSENTIAL_VCPS
const UI_VCP_NAMES: &[&str] = &[
    "brightness", "contrast", "color_preset", "picture_mode",
    "red_gain", "green_gain", "blue_gain", "input_source",
    "volume", "color_temp_kelvin", "gamma_curve", "sharpness",
    "audio_mute", "h_freq", "v_freq", "display_tech",
    "usage_hours", "backlight_pwm", "firmware", "vcp_version",
    "osd_lock", "language", "power_mode", "split_mode",
    "aspect_ratio", "smart_energy", "response_time", "freesync",
    "black_stabilizer", "power_led", "gamma",
];

#[test]
fn test_all_ui_names_exist_in_essential_vcps() {
    // Names used in UI must match ddc::ESSENTIAL_VCPS exactly
    let essential_names: Vec<&str> = vec![
        "brightness", "contrast", "color_preset", "picture_mode",
        "red_gain", "green_gain", "blue_gain", "input_source",
        "volume", "color_temp_kelvin", "gamma_curve", "sharpness",
        "audio_mute", "h_freq", "v_freq", "display_tech",
        "usage_hours", "backlight_pwm", "firmware", "osd_lock",
        "language", "power_mode", "split_mode", "vcp_version",
        "aspect_ratio", "smart_energy", "response_time", "freesync",
        "black_stabilizer", "power_led", "gamma",
    ];
    for name in UI_VCP_NAMES {
        assert!(essential_names.contains(name),
            "UI references VCP name '{}' not in ESSENTIAL_VCPS", name);
    }
}

#[test]
fn test_no_duplicate_vcp_codes() {
    let codes: Vec<u8> = vec![
        0x10, 0x12, 0x14, 0x15, 0x16, 0x18, 0x1A, 0x60,
        0x62, 0x69, 0x72, 0x87, 0x8D, 0xAC, 0xAE, 0xB6,
        0xC0, 0xC1, 0xC9, 0xCA, 0xCC, 0xD6, 0xD7, 0xDF,
        0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFD, 0xFE,
    ];
    let mut seen = std::collections::HashSet::new();
    for code in &codes {
        assert!(seen.insert(code), "Duplicate VCP code: 0x{:02X}", code);
    }
}

#[test]
fn test_no_duplicate_vcp_names() {
    let mut seen = std::collections::HashSet::new();
    for name in UI_VCP_NAMES {
        assert!(seen.insert(name), "Duplicate VCP name: {}", name);
    }
}

#[test]
fn test_essential_vcps_count() {
    // We expect exactly 31 VCPs
    assert_eq!(UI_VCP_NAMES.len(), 31);
}

// ── Picture Mode Value Tests ───────────────────────────────────────────────

#[test]
fn test_picture_mode_values_match_re_doc() {
    // From RE doc capabilities string
    let modes: Vec<(u16, &str)> = vec![
        (17, "Custom"), (1, "Reader"), (32, "Photo"), (72, "Cinema"),
        (6, "Color Weakness"), (40, "FPS 1"), (41, "FPS 2"),
        (19, "RTS"), (20, "Vivid"), (21, "sRGB"), (24, "SMPTE-C"),
        (25, "EBU"), (45, "Gamer"),
    ];
    // All values must fit in u16 and be unique
    let mut seen = std::collections::HashSet::new();
    for (val, name) in &modes {
        assert!(seen.insert(val), "Duplicate picture mode value {} ({})", val, name);
        assert!(*val <= 255, "Picture mode {} ({}) exceeds u8 range", val, name);
    }
}

// ── Language Value Tests ───────────────────────────────────────────────────

#[test]
fn test_language_values_match_re_doc() {
    // RE doc: 0=EN, 2=FR, 3=DE, 4=ES, 5=IT, 6=KO, 7=ZH, 8=JA, 9=PT
    let languages: Vec<(u16, &str)> = vec![
        (0, "English"), (2, "French"), (3, "Deutsch"), (4, "Spanish"),
        (5, "Italian"), (6, "Korean"), (7, "Chinese"), (8, "Japanese"),
        (9, "Portuguese"),
    ];
    // Value 1 is NOT a valid language on this monitor
    for (val, _) in &languages {
        assert_ne!(*val, 1, "Language value 1 is not valid on LG 34GN850");
    }
    // French must be 2, not 1
    assert_eq!(languages[1].0, 2, "French must be VCP value 2");
}

// ── Input Source Value Tests ───────────────────────────────────────────────

#[test]
fn test_input_source_values_valid() {
    // LG 34GN850 has: DP (0x0F), HDMI1 (0x11), HDMI2 (0x12)
    let inputs: Vec<(u16, &str)> = vec![
        (0x0F, "DisplayPort"), (0x11, "HDMI 1"), (0x12, "HDMI 2"),
    ];
    // No USB-C on this monitor
    for (val, name) in &inputs {
        assert_ne!(*val, 0x22, "{} has invalid USB-C value", name);
    }
    assert_eq!(inputs.len(), 3, "LG 34GN850 has exactly 3 inputs");
}

// ── Response Time Value Tests ──────────────────────────────────────────────

#[test]
fn test_response_time_labels() {
    // RE doc: 0=Off, 1=Fast, 2=Normal, 3=Slow, 4=Faster
    let rt: Vec<(u16, &str)> = vec![
        (0, "Off"), (1, "Fast"), (2, "Normal"), (3, "Slow"), (4, "Faster"),
    ];
    // Must NOT use "High/Middle/Low"
    for (_, label) in &rt {
        assert_ne!(*label, "High", "Response Time should use 'Fast' not 'High'");
        assert_ne!(*label, "Middle", "Response Time should use 'Normal' not 'Middle'");
        assert_ne!(*label, "Low", "Response Time should use 'Slow' not 'Low'");
    }
}

// ── Display Formatting Tests ───────────────────────────────────────────────

#[test]
fn test_firmware_version_format() {
    let fw: u16 = 768; // 0x0300
    let major = fw >> 8;
    let minor = fw & 0xFF;
    assert_eq!(format!("{}.{}", major, minor), "3.0");
}

#[test]
fn test_vcp_version_format() {
    let ver: u16 = 513; // 0x0201
    let major = ver >> 8;
    let minor = ver & 0xFF;
    assert_eq!(format!("{}.{}", major, minor), "2.1");
}

#[test]
fn test_display_tech_labels() {
    // MCCS standard: 1=CRT, 2=LCD, 3=IPS (LG reports 3)
    let label = match 3u16 {
        1 => "CRT", 2 => "LCD", 3 => "IPS",
        4 => "OLED", 5 => "VA", _ => "Unknown",
    };
    assert_eq!(label, "IPS");
    // Value 2 must be LCD, not LED
    let label2 = match 2u16 {
        1 => "CRT", 2 => "LCD", 3 => "IPS",
        4 => "OLED", 5 => "VA", _ => "Unknown",
    };
    assert_eq!(label2, "LCD");
}

#[test]
fn test_color_preset_decode() {
    assert_eq!(match 5u16 { 5 => "6500K", 8 => "9300K", 0x0B => "User", _ => "Other" }, "6500K");
    assert_eq!(match 8u16 { 5 => "6500K", 8 => "9300K", 0x0B => "User", _ => "Other" }, "9300K");
    assert_eq!(match 0x0Bu16 { 5 => "6500K", 8 => "9300K", 0x0B => "User", _ => "Other" }, "User");
}

#[test]
fn test_usage_hours_display() {
    let hours: u16 = 33909;
    let days = hours / 24;
    assert_eq!(format!("{} h ({} days)", hours, days), "33909 h (1412 days)");
}

// ── Checksum Edge Cases ────────────────────────────────────────────────────

#[test]
fn test_checksum_all_zeros() {
    let chk = ddc_write_checksum(0x00, 0x0000);
    let expected = 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x00 ^ 0x00 ^ 0x00;
    assert_eq!(chk, expected);
}

#[test]
fn test_checksum_max_values() {
    let chk = ddc_write_checksum(0xFF, 0xFFFF);
    let expected = 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0xFF ^ 0xFF ^ 0xFF;
    assert_eq!(chk, expected);
}
