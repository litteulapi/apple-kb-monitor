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
    let fd = unsafe { libc::socket(31, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 1) };
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

    // Read until our own command reply shows up. The control channel also
    // delivers unrelated events (index added, class changed, ...) that used to
    // be mistaken for the reply and made the read fail.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
    let mut result = None;
    while std::time::Instant::now() < deadline {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) { continue; }
            break; // timeout or socket error
        }
        match parse_conn_info_reply(&buf[..n as usize]) {
            Reply::NotOurs => continue,
            Reply::Failed => break,
            Reply::Info(r) => { result = Some(r); break; }
        }
    }
    unsafe { libc::close(fd) };
    result
}

/// Outcome of inspecting one MGMT event packet.
#[derive(Debug, PartialEq)]
enum Reply {
    /// Event for something else (other opcode / other event type).
    NotOurs,
    /// Our command was answered with a non-zero status or unusable values.
    Failed,
    /// (rssi_dbm, tx_power_dbm)
    Info((i8, i8)),
}

/// BlueZ reports 127 when RSSI / TX power is not available.
const MGMT_VALUE_INVALID: i8 = 127;

/// Parse a MGMT event: header `event(u16) index(u16) len(u16)` followed by
/// `opcode(u16) status(u8)` and, for Get Connection Info, `addr[6] type(u8)
/// rssi tx_power max_tx_power`.
fn parse_conn_info_reply(pkt: &[u8]) -> Reply {
    if pkt.len() < 9 { return Reply::NotOurs; }
    let event = u16::from_le_bytes([pkt[0], pkt[1]]);
    let opcode = u16::from_le_bytes([pkt[6], pkt[7]]);
    // MGMT_EV_CMD_COMPLETE = 1, MGMT_EV_CMD_STATUS = 2
    if (event != 0x0001 && event != 0x0002) || opcode != 0x0031 {
        return Reply::NotOurs;
    }
    if pkt[8] != 0x00 || event == 0x0002 || pkt.len() < 19 {
        return Reply::Failed;
    }
    let rssi = pkt[16] as i8;
    let tx = pkt[17] as i8;
    if rssi == MGMT_VALUE_INVALID || tx == MGMT_VALUE_INVALID {
        return Reply::Failed;
    }
    Reply::Info((rssi, tx))
}

/// Parse a colon-separated MAC string into 6 bytes.
fn parse_mac(mac: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut octets = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        // Exactly two hex digits: from_str_radix alone accepts "+A" and "1".
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        octets[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(opcode: u16, status: u8, rssi: i8, tx: i8) -> Vec<u8> {
        let mut p = vec![0u8; 19];
        p[0..2].copy_from_slice(&1u16.to_le_bytes());
        p[6..8].copy_from_slice(&opcode.to_le_bytes());
        p[8] = status;
        p[16] = rssi as u8;
        p[17] = tx as u8;
        p[18] = 4;
        p
    }

    #[test]
    fn valid_reply() {
        assert_eq!(parse_conn_info_reply(&complete(0x31, 0, -42, 4)), Reply::Info((-42, 4)));
    }

    #[test]
    fn foreign_events_are_skipped_not_failed() {
        assert_eq!(parse_conn_info_reply(&complete(0x30, 0, -42, 4)), Reply::NotOurs);
        let mut ev = complete(0x31, 0, -42, 4);
        ev[0] = 0x04; // some other event type
        assert_eq!(parse_conn_info_reply(&ev), Reply::NotOurs);
        assert_eq!(parse_conn_info_reply(&[1, 0, 0]), Reply::NotOurs);
        assert_eq!(parse_conn_info_reply(&[]), Reply::NotOurs);
    }

    #[test]
    fn failures_and_invalid_values() {
        assert_eq!(parse_conn_info_reply(&complete(0x31, 0x02, 0, 0)), Reply::Failed); // not connected
        assert_eq!(parse_conn_info_reply(&complete(0x31, 0, 127, 4)), Reply::Failed);
        assert_eq!(parse_conn_info_reply(&complete(0x31, 0, -40, 127)), Reply::Failed);
        let truncated = complete(0x31, 0, -40, 4)[..15].to_vec();
        assert_eq!(parse_conn_info_reply(&truncated), Reply::Failed);
        let mut st = complete(0x31, 0, -40, 4);
        st[0] = 2; // CMD_STATUS
        assert_eq!(parse_conn_info_reply(&st), Reply::Failed);
    }

    #[test]
    fn mac_parsing_is_strict() {
        assert_eq!(parse_mac("AA:bb:0C:dd:EE:01"), Some([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0x01]));
        assert_eq!(parse_mac("+A:bb:0C:dd:EE:01"), None);
        assert_eq!(parse_mac("A:bb:0C:dd:EE:01"), None);
        assert_eq!(parse_mac("AA:bb:0C:dd:EE"), None);
        assert_eq!(parse_mac("AA:bb:0C:dd:EE:01:02"), None);
        assert_eq!(parse_mac("ZZ:bb:0C:dd:EE:01"), None);
        assert_eq!(parse_mac(""), None);
    }

    #[test]
    fn read_rssi_bad_mac_is_none() {
        assert_eq!(read_rssi("not a mac"), None);
    }
}
