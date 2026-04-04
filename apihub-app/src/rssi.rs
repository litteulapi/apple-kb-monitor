//! BlueZ MGMT API — read RSSI and TX power for a connected BT device.
//!
//! Replaces the C `rssi-helper` binary with a pure Rust equivalent.
//! Uses AF_BLUETOOTH + BTPROTO_HCI + HCI_CHANNEL_CONTROL to send
//! MGMT opcode 0x0031 (Get Connection Info) and parse the response.
//!
//! Requires CAP_NET_ADMIN or root. Returns `None` on any failure.

/// Read RSSI and TX power for a Bluetooth device via BlueZ MGMT API.
///
/// `mac` must be colon-separated hex, e.g. `"AA:BB:CC:DD:EE:FF"`.
/// Returns `(rssi_dbm, tx_power_dbm)` or `None` if the socket or
/// the MGMT command fails (device not connected, no privileges, etc.).
pub fn read_rssi(mac: &str) -> Option<(i8, i8)> {
    // Parse MAC address
    let octets = parse_mac(mac)?;

    // AF_BLUETOOTH = 31, BTPROTO_HCI = 1
    let fd = unsafe { libc::socket(31, libc::SOCK_RAW, 1) };
    if fd < 0 {
        return None;
    }

    // Bind to HCI_CHANNEL_CONTROL (3), HCI_DEV_NONE (0xFFFF)
    #[repr(C)]
    struct SockaddrHci {
        family: libc::sa_family_t,
        dev: u16,
        channel: u16,
    }

    let sa = SockaddrHci {
        family: 31,
        dev: 0xFFFF,
        channel: 3, // HCI_CHANNEL_CONTROL
    };

    if unsafe {
        libc::bind(
            fd,
            &sa as *const SockaddrHci as *const libc::sockaddr,
            std::mem::size_of::<SockaddrHci>() as libc::socklen_t,
        )
    } < 0
    {
        unsafe { libc::close(fd) };
        return None;
    }

    // 200ms receive timeout (was 2s — reduces poll thread blocking)
    let tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 200_000,
    };
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const libc::timeval as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }

    // Build MGMT request: header (6 bytes) + command params (7 bytes) = 13 bytes
    //
    // Header: opcode(u16) = 0x0031, index(u16) = 0x0000, param_len(u16) = 7
    // Params: addr[6] (LE byte order) + addr_type(u8) = 0 (BR/EDR)
    let mut buf = [0u8; 256];

    // opcode 0x0031 (Get Connection Info), little-endian
    buf[0] = 0x31;
    buf[1] = 0x00;
    // index 0x0000 (first controller)
    buf[2] = 0x00;
    buf[3] = 0x00;
    // param length = 7
    buf[4] = 0x07;
    buf[5] = 0x00;
    // MAC in reverse byte order (BlueZ MGMT convention)
    buf[6] = octets[5];
    buf[7] = octets[4];
    buf[8] = octets[3];
    buf[9] = octets[2];
    buf[10] = octets[1];
    buf[11] = octets[0];
    // addr_type = 0 (BR/EDR public)
    buf[12] = 0x00;

    let sent = unsafe { libc::send(fd, buf.as_ptr() as *const libc::c_void, 13, 0) };
    if sent < 0 {
        unsafe { libc::close(fd) };
        return None;
    }

    // Read response
    let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
    unsafe { libc::close(fd) };

    if n < 19 {
        return None;
    }
    let n = n as usize;

    // Validate: first 2 bytes = MGMT_EV_CMD_COMPLETE (0x0001)
    let ev_opcode = u16::from_le_bytes([buf[0], buf[1]]);
    if ev_opcode != 0x0001 {
        return None;
    }

    // Status byte at offset 8 must be 0 (success)
    if n <= 8 || buf[8] != 0x00 {
        return None;
    }

    // RSSI at offset 16, TX power at offset 17
    if n < 18 {
        return None;
    }
    let rssi = buf[16] as i8;
    let tx_power = buf[17] as i8;

    Some((rssi, tx_power))
}

/// Parse a colon-separated MAC string into 6 bytes.
fn parse_mac(mac: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut octets = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        octets[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(octets)
}
