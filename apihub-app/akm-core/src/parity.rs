//! Parity with Apple: the one command macOS sends to this keyboard.
//!
//! Scope, by decision of the manager: only what Apple's own driver really
//! sends to the A1314, with the same bytes and in the same context, never more.
//! Disassembly of macOS 26.5 (docs/RE-GHIDRA-KEXT.md, RE-GHIDRA-IOBLUETOOTH.md,
//! RE-PILOTE-MACOS.md §5): at every shutdown and every restart
//! `IOAppleBluetoothHIDDriver::handleShutdown` / `handleRestart` call
//! `willShutdown()`, which does `setExtendedReport("WillShutdown", NULL, 0)`:
//! a Feature report whose buffer is the id alone, `0x40`; on the Bluetooth link
//! `53 40`. Nothing else: no HID_CONTROL, no name, no reset, no `0x4A`.
//!
//! The rules enforced here (each one tested with a spy, never on the hardware):
//!
//! * only if `[apple] will_shutdown` is on (default **on**, it is what Apple sends);
//! * only if the keyboard is connected;
//! * **once** per run ([`WriteSession`]), whichever path asks (logind
//!   `PrepareForShutdown`, `akmctl shutdown-notify`, the systemd unit);
//! * with the circuit breaker of the safe read policy ([`Breaker`]) and the
//!   1 s spacing after the previous hardware access ([`MIN_GAP`]);
//! * through the register map only ([`crate::registry::check_write`]: id `0x40`,
//!   no data) and the single hardware door
//!   ([`crate::hidraw::hid_write_feature`]).
//!
//! A failure is reported and never retried; it never blocks the shutdown.

use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::read_policy::{wait_before, Breaker};
use crate::registry::{Direction, WriteSession};

/// The only id written: `WillShutdown`.
pub const WILL_SHUTDOWN_ID: u8 = 0x40;

/// Where the report goes: the real hidraw node, or a spy in the tests.
/// `report` is the exact buffer handed to the kernel (id first).
pub trait FeatureSink {
    fn set_feature(&self, report: &[u8]) -> io::Result<()>;
}

/// What happened to a `WillShutdown` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The report was handed to the kernel (`53 40` on the wire).
    Sent,
    /// `[apple] will_shutdown = false`.
    Disabled,
    /// The keyboard is not connected: nothing to tell.
    NotConnected,
    /// Already sent in this run (one `WillShutdown` per shutdown).
    AlreadySent,
    /// The circuit breaker is open: the keyboard stopped answering.
    BreakerOpen,
    /// Another reader holds the hardware lock.
    Busy,
    /// No hidraw node could be opened.
    NoNode,
    /// The kernel refused or the write failed (not retried).
    Failed(String),
}

impl Outcome {
    pub fn is_sent(&self) -> bool {
        matches!(self, Self::Sent)
    }

    /// One-line explanation, used by the daemon log and `akmctl shutdown-notify`.
    pub fn describe(&self) -> String {
        match self {
            Self::Sent => "WillShutdown sent (Feature 0x40, wire 53 40)".into(),
            Self::Disabled => "WillShutdown disabled ([apple] will_shutdown = false): nothing sent".into(),
            Self::NotConnected => "keyboard not connected: nothing sent".into(),
            Self::AlreadySent => "WillShutdown already sent in this run: nothing sent".into(),
            Self::BreakerOpen => "circuit breaker open (the keyboard stopped answering): nothing sent".into(),
            Self::Busy => "another reader holds the keyboard lock: nothing sent".into(),
            Self::NoNode => "no hidraw node for the keyboard: nothing sent".into(),
            Self::Failed(e) => format!("WillShutdown write failed (not retried): {e}"),
        }
    }
}

/// Send `WillShutdown` if every condition holds. `last_hw` is the instant of
/// the previous request to the keyboard, `sleep` waits for the 1 s spacing
/// (a spy in the tests). Pure orchestration: the bytes only ever reach `sink`
/// after the session and the register map agreed.
pub fn will_shutdown(
    enabled: bool,
    connected: bool,
    sink: &dyn FeatureSink,
    session: &mut WriteSession,
    breaker: &Mutex<Breaker>,
    last_hw: Option<Instant>,
    sleep: &mut dyn FnMut(Duration),
) -> Outcome {
    if !enabled {
        return Outcome::Disabled;
    }
    if !connected {
        return Outcome::NotConnected;
    }
    if session.is_used() {
        return Outcome::AlreadySent;
    }
    let allowed = breaker.lock().unwrap_or_else(|e| e.into_inner()).allow();
    if !allowed {
        return Outcome::BreakerOpen;
    }
    let wait = wait_before(last_hw, Instant::now());
    if !wait.is_zero() {
        sleep(wait);
    }
    // The id alone: no data, exactly what Apple's driver hands to the stack.
    if let Err(e) = session.authorize(WILL_SHUTDOWN_ID, Direction::Feature, &[]) {
        return Outcome::Failed(e.to_string());
    }
    let r = sink.set_feature(&[WILL_SHUTDOWN_ID]);
    breaker
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(r.is_ok());
    match r {
        Ok(()) => Outcome::Sent,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Spy {
        log: RefCell<Vec<Vec<u8>>>,
        fail: bool,
    }
    impl FeatureSink for Spy {
        fn set_feature(&self, report: &[u8]) -> io::Result<()> {
            self.log.borrow_mut().push(report.to_vec());
            if self.fail {
                Err(io::Error::from_raw_os_error(libc::EIO))
            } else {
                Ok(())
            }
        }
    }

    fn run(spy: &Spy, enabled: bool, connected: bool, s: &mut WriteSession, b: &Mutex<Breaker>, last: Option<Instant>) -> (Outcome, Vec<Duration>) {
        let mut waits = Vec::new();
        let o = will_shutdown(enabled, connected, spy, s, b, last, &mut |d| waits.push(d));
        (o, waits)
    }

    #[test]
    fn sends_exactly_one_byte_0x40_once() {
        let spy = Spy::default();
        let (mut s, b) = (WriteSession::new(), Mutex::new(Breaker::new()));
        let (o, _) = run(&spy, true, true, &mut s, &b, None);
        assert_eq!(o, Outcome::Sent);
        assert_eq!(*spy.log.borrow(), vec![vec![0x40]]);
        // a second request, from any path, is refused and sends nothing
        let (o, _) = run(&spy, true, true, &mut s, &b, None);
        assert_eq!(o, Outcome::AlreadySent);
        assert_eq!(spy.log.borrow().len(), 1);
    }

    #[test]
    fn disabled_or_disconnected_sends_nothing_and_keeps_the_session() {
        let spy = Spy::default();
        let (mut s, b) = (WriteSession::new(), Mutex::new(Breaker::new()));
        assert_eq!(run(&spy, false, true, &mut s, &b, None).0, Outcome::Disabled);
        assert_eq!(run(&spy, true, false, &mut s, &b, None).0, Outcome::NotConnected);
        assert!(spy.log.borrow().is_empty() && !s.is_used());
    }

    #[test]
    fn open_breaker_blocks_the_write() {
        let spy = Spy::default();
        let (mut s, b) = (WriteSession::new(), Mutex::new(Breaker::new()));
        for _ in 0..crate::read_policy::TRIP_AFTER {
            b.lock().unwrap().record(false);
        }
        assert_eq!(run(&spy, true, true, &mut s, &b, None).0, Outcome::BreakerOpen);
        assert!(spy.log.borrow().is_empty() && !s.is_used());
        // a sign of life re-arms ONE probe: the write may go, and closes the breaker
        b.lock().unwrap().alive();
        assert_eq!(run(&spy, true, true, &mut s, &b, None).0, Outcome::Sent);
        assert!(!b.lock().unwrap().is_open());
    }

    #[test]
    fn spacing_of_one_second_after_the_previous_access() {
        let spy = Spy::default();
        let (mut s, b) = (WriteSession::new(), Mutex::new(Breaker::new()));
        let (_, waits) = run(&spy, true, true, &mut s, &b, Some(Instant::now()));
        assert_eq!(waits.len(), 1);
        assert!(waits[0] > Duration::from_millis(900) && waits[0] <= crate::read_policy::MIN_GAP);
        // long ago or never: no wait
        let spy = Spy::default();
        let mut s = WriteSession::new();
        let old = Instant::now().checked_sub(Duration::from_secs(5));
        assert!(run(&spy, true, true, &mut s, &b, old).1.is_empty());
    }

    #[test]
    fn a_failed_write_is_reported_counted_and_never_retried() {
        let spy = Spy { fail: true, ..Spy::default() };
        let (mut s, b) = (WriteSession::new(), Mutex::new(Breaker::new()));
        let (o, _) = run(&spy, true, true, &mut s, &b, None);
        assert!(matches!(o, Outcome::Failed(_)));
        assert!(o.describe().contains("not retried"));
        assert_eq!(run(&spy, true, true, &mut s, &b, None).0, Outcome::AlreadySent);
        assert_eq!(spy.log.borrow().len(), 1, "one attempt only");
    }

    #[test]
    fn every_outcome_has_a_description() {
        for o in [
            Outcome::Sent,
            Outcome::Disabled,
            Outcome::NotConnected,
            Outcome::AlreadySent,
            Outcome::BreakerOpen,
            Outcome::Busy,
            Outcome::NoNode,
            Outcome::Failed("x".into()),
        ] {
            assert!(!o.describe().is_empty());
        }
        assert!(Outcome::Sent.is_sent() && !Outcome::Disabled.is_sent());
    }
}
