//! Conformance suite of the model (#251): one test per Apple rule (the rule
//! and its source are in the test's comment), full scenarios on a simulated
//! clock, and properties (proptest).

use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A connected, ready model (SET_PROTOCOL answered at `t0`).
fn ready(t0: Instant) -> AppleModel {
    let mut m = AppleModel::new(true);
    m.connected();
    m.send(Request::SetProtocol, t0).unwrap();
    m.complete(Outcome::Answered, t0);
    assert!(m.link.is_ready());
    m
}

/// Send one request at `t` and finish it with `o`.
fn exchange(m: &mut AppleModel, req: Request, o: Outcome, t: Instant) -> Result<Timeouts, Refusal> {
    let r = m.send(req, t)?;
    m.complete(o, t);
    Ok(r)
}

const GET47: Request = Request::GetFeature(ids::BATTERY_PERCENT);

// ── R8: the table ──────────────────────────────────────────────────────────

/// R8: the values of the table, as read in the sources cited on each field.
#[test]
fn r8_table_holds_apples_values() {
    assert_eq!(APPLE.set_protocol_timeout, ms(10_000));
    assert_eq!(APPLE.first_battery_read, ms(60_000));
    assert_eq!(APPLE.battery_period, ms(14_400_000));
    assert_eq!(APPLE.battery_retry, ms(3_600_000));
    assert_eq!(APPLE.report_timeout, ms(3_500));
    assert_eq!(APPLE.guard_margin, ms(1_000));
    assert_eq!(APPLE.sleep_report_timeout, ms(0x1194));
    assert_eq!(APPLE.sleep_guard, ms(0x157C));
    assert_eq!(APPLE.min_gap, ms(0x3e8));
    assert_eq!(APPLE.trip_after, 3);
    assert_eq!(APPLE.counter_after_sleep, 1);
    assert_eq!(APPLE.refusal_max, crate::decode::DecodeOptions::default().slow_failure);
    assert_eq!((APPLE.display_full_from, APPLE.display_curve_from), (0x36, 0x15));
    assert_eq!(APPLE.display_slope, 2.4375);
    assert_eq!((APPLE.percent_max, APPLE.battery_state_max), (100, 3));
    assert!(APPLE.disconnect_call_timeout <= s(5));
    assert_eq!(BATTERY_READ, [Request::GetFeature(0x47), Request::GetInput(0x30)]);
    assert_eq!((ids::WILL_SHUTDOWN, ids::KEYBOARD_STATUS, ids::POWERED_ON_BIT), (0x40, 0x13, 0x02));
    // every rule names its source and its test
    for r in RULES {
        assert!(r.source.starts_with("[décompilé]") || r.source.starts_with("[plist]"), "{}", r.id);
        assert!(r.test.starts_with(&r.id[..2].to_ascii_lowercase()), "{}", r.id);
    }
    assert_eq!(RULES.len(), 10);
}

// ── R1 ─────────────────────────────────────────────────────────────────────

/// R1 — [décompilé] `IOBluetoothHIDDriver::deviceReady` `0xfffffe000a35a588`:
/// SET_PROTOCOL(Report) with a 10 s wait; on failure the device is never
/// declared ready: no generic command, no battery read.
#[test]
fn r1_set_protocol_failure_means_never_ready() {
    let t0 = Instant::now();
    let mut m = AppleModel::new(true);
    assert_eq!(m.send(GET47, t0), Err(Refusal::NotConnected));
    m.connected();
    assert_eq!(m.link.state(), LinkState::AwaitingProtocol);
    // nothing but SET_PROTOCOL before readiness
    assert_eq!(m.send(GET47, t0), Err(Refusal::NotReady));
    let t = m.send(Request::SetProtocol, t0).unwrap();
    assert_eq!(t, Timeouts { wait: ms(10_000), guard: ms(11_000) });
    // the watchdog expires at the guard, not before
    m.poll(t0 + ms(10_999));
    assert_eq!(m.in_flight(), Some(Request::SetProtocol));
    m.poll(t0 + ms(11_000));
    assert_eq!(m.in_flight(), None);
    assert_eq!(m.link.state(), LinkState::NotReady);
    // never ready: no battery timer, every request refused, also later
    let later = t0 + s(24 * 3600);
    assert!(!m.link.battery_due(later));
    assert_eq!(m.link.next_battery(), None);
    for req in [GET47, Request::SetProtocol, Request::SetFeature(0x40), Request::Output] {
        assert_eq!(m.send(req, later), Err(Refusal::NotReady));
    }
    m.link.protocol_result(true, later);
    assert_eq!(m.link.state(), LinkState::NotReady, "a late answer changes nothing");
    // a refused SET_PROTOCOL (HANDSHAKE error) is not a success either
    let mut r = AppleModel::new(true);
    r.connected();
    exchange(&mut r, Request::SetProtocol, Outcome::Refused, t0).unwrap();
    assert_eq!(r.link.state(), LinkState::NotReady);
    // a SET_PROTOCOL is never sent again once ready
    let mut ok = ready(t0);
    assert_eq!(ok.send(Request::SetProtocol, t0 + s(5)), Err(Refusal::NotReady));
}

/// R1 — [décompilé] `startBatteryUpdate` `0xfffffe000a356034` (60 000 ms),
/// `IOAppleBluetoothHIDDriver::deviceReady` `0xfffffe000a354f2c`
/// (14 400 000 ms), `batteryLevelTimerFired` `0xfffffe000a355034`
/// (3 600 000 ms after a failure of either read).
#[test]
fn r1_battery_schedule_60s_then_4h_or_1h() {
    let t0 = Instant::now();
    let mut l = LinkModel::new();
    assert!(!l.begin_cycle(t0));
    l.connected();
    assert_eq!(l.next_battery(), None, "not before SET_PROTOCOL");
    l.protocol_result(true, t0);
    assert_eq!(l.next_battery(), Some(t0 + s(60)));
    assert_eq!(l.next_deadline(), Some(t0 + s(60)));
    assert!(!l.battery_due(t0 + ms(59_999)));
    assert!(!l.begin_cycle(t0 + ms(59_999)));
    assert!(l.begin_cycle(t0 + s(60)));
    assert!(l.cycle_running());
    assert!(!l.battery_due(t0 + s(61)), "one read at a time");
    assert!(!l.begin_cycle(t0 + s(61)));
    assert_eq!(l.next_deadline(), None);
    // both succeeded: 4 h
    l.end_cycle(true, true, t0 + s(62));
    assert!(!l.cycle_running());
    assert_eq!(l.next_battery(), Some(t0 + s(62) + s(4 * 3600)));
    // 0x47 failed: 1 h
    let t1 = t0 + s(62 + 4 * 3600);
    assert!(l.begin_cycle(t1));
    l.end_cycle(false, true, t1);
    assert_eq!(l.next_battery(), Some(t1 + s(3600)));
    // 0x30 failed: 1 h too
    let t2 = t1 + s(3600);
    assert!(l.begin_cycle(t2));
    l.end_cycle(true, false, t2);
    assert_eq!(l.next_battery(), Some(t2 + s(3600)));
    // an end without a running read changes nothing
    l.end_cycle(true, true, t2 + s(5));
    assert_eq!(l.next_battery(), Some(t2 + s(3600)));
    // disconnection stops everything
    l.disconnected();
    assert_eq!((l.state(), l.next_battery()), (LinkState::Disconnected, None));
    assert!(l.check().is_ok() && LinkModel::default() == LinkModel::new());
}

// ── R2 ─────────────────────────────────────────────────────────────────────

/// R2 — [décompilé] `batteryLevelTimerFired` -> `updateBatteryLevel`
/// (GET Feature `0x47`, `43 47`) then `getBatteryState` (GET Input `0x30`,
/// `41 30`), each a 2-byte request without size; RE-PILOTE-MACOS §4.
#[test]
fn r2_read_is_0x47_then_input_0x30() {
    let t0 = Instant::now();
    let mut m = ready(t0);
    let at = t0 + APPLE.first_battery_read;
    assert!(m.link.begin_cycle(at));
    let mut t = at;
    for req in BATTERY_READ {
        exchange(&mut m, req, Outcome::Answered, t).unwrap();
        t += APPLE.min_gap;
    }
    m.link.end_cycle(true, true, t);
    assert_eq!(m.link.next_battery(), Some(t + APPLE.battery_period));
    assert_eq!(m.battery_percent(0x63), 99);
}

/// R2 — [plist] `GetReportTimeoutMS` = 3500; [décompilé] `waitForData`
/// `0xfffffe000a35daa0` (guard = timeout + 1000); one request at a time
/// (`IOCommandGate`); IOBluetooth passes 1000 ms (`mov w5,#0x3e8`).
#[test]
fn r2_timeouts_spacing_and_single_request() {
    let t0 = Instant::now();
    let mut m = ready(t0);
    let t1 = t0 + s(1);
    let t = m.send(GET47, t1).unwrap();
    assert_eq!(t, Timeouts { wait: ms(3_500), guard: ms(4_500) });
    assert_eq!(m.next_deadline(), Some(t1 + ms(4_500)));
    assert_eq!(m.send(GET47, t1 + s(2)), Err(Refusal::InFlight));
    m.poll(t1 + ms(4_499));
    assert_eq!(m.in_flight(), Some(GET47));
    m.complete(Outcome::Answered, t1 + ms(200));
    // 1 s between two requests, whatever their outcome
    assert_eq!(m.send(GET47, t1 + ms(400)), Err(Refusal::TooSoon(ms(600))));
    assert_eq!(m.send(GET47, t1 + ms(999)), Err(Refusal::TooSoon(ms(1))));
    assert!(m.send(GET47, t1 + ms(1000)).is_ok());
    // classification of a Linux answer against the guard
    let g = ms(4_500);
    let ok = |b: &[u8]| -> io::Result<Vec<u8>> { Ok(b.to_vec()) };
    assert_eq!(Outcome::classify(&ok(&[0x47, 99]), 0x47, g, g), Outcome::Answered);
    assert_eq!(Outcome::classify(&ok(&[0x47, 99]), 0x47, g + ms(1), g), Outcome::Timeout);
    // complete without a request in flight: ignored
    let mut idle = ready(t0);
    idle.complete(Outcome::Timeout, t0);
    assert_eq!(idle.breaker.counter(), 0);
}

// ── R3 ─────────────────────────────────────────────────────────────────────

/// R3 — [décompilé] `waitForData` / `waitForHandshake` `0xfffffe000a35e478` /
/// `waitForOkToSend` `0xfffffe000a35ec50`: one counter for GET and SET; at
/// the 3rd silence, flag `+0xA0` (then `getReport`, `setReport` and
/// `sendData` `0xfffffe000a35b9cc` emit nothing: LED, SET_PROTOCOL and
/// HID_CONTROL included) and `SetHIDDriverReady(false)`: bluetoothd
/// disconnects, once.
#[test]
fn r3_three_silences_block_everything_and_ask_once() {
    let t0 = Instant::now();
    let mut m = ready(t0);
    // GET, SET and GET silences share the counter
    let seq = [GET47, Request::SetFeature(0x40), GET47];
    for (i, req) in seq.into_iter().enumerate() {
        let t = t0 + s(10 * (i as u64 + 1));
        m.send(req, t).unwrap();
        m.poll(t + APPLE.report_timeout + APPLE.guard_margin);
        assert_eq!(m.breaker.counter(), i as u32 + 1);
        assert_eq!(m.breaker.is_open(), i == 2);
    }
    assert_eq!(m.take_effects(), vec![Effect::RequestDisconnect]);
    assert!(m.take_effects().is_empty());
    let t = t0 + s(100);
    for req in [GET47, Request::GetInput(0x30), Request::SetFeature(0x40), Request::Output, Request::HidControl(3), Request::SetProtocol] {
        assert_eq!(m.send(req, t), Err(Refusal::BreakerOpen), "{req:?}");
    }
    assert_eq!(m.will_shutdown(t), Err(Refusal::BreakerOpen));
    // the battery timer still fires, its read sends nothing and retries in 1 h
    let mut b = Breaker::new();
    for _ in 0..3 {
        b.record(false);
    }
    assert!(b.is_open() && !b.allow() && !b.would_allow());
    assert!(b.take_disconnect_request());
    assert!(!b.take_disconnect_request(), "once");
    // further silences (probes) never ask again in the same connection
    b.alive();
    assert!(b.would_allow() && b.allow() && !b.allow());
    assert!(!b.record_outcome(Outcome::Timeout));
    assert!(!b.take_disconnect_request());
    // a new connection rebuilds the driver: closed, counter 0, may ask again
    b.reset();
    assert_eq!(b, Breaker::new());
    assert_eq!(Breaker::default(), Breaker::new());
    // disabled ([apple] disconnect_on_breaker = false): blocked, no request
    let mut quiet = AppleModel::new(false);
    quiet.connected();
    exchange(&mut quiet, Request::SetProtocol, Outcome::Answered, t0).unwrap();
    for i in 1..=3 {
        exchange(&mut quiet, GET47, Outcome::Timeout, t0 + s(i)).unwrap();
    }
    assert!(quiet.breaker.is_open());
    assert!(quiet.take_effects().is_empty());
}

/// R3 — [décompilé] `DecodedHandshake` `0xfffffe000a35e324`: a HANDSHAKE
/// refusal is an answer (counter and flag to 0); `processControlData`
/// `0xfffffe000a35c898`: a DATA of another id is ignored and the request
/// expires (a silence).
#[test]
fn r3_refusal_is_an_answer_wrong_id_is_a_silence() {
    let mut b = Breaker::new();
    b.record(false);
    b.record(false);
    assert!(!b.record_outcome(Outcome::Refused));
    assert_eq!(b.counter(), 0);
    for o in [Outcome::WrongId, Outcome::WrongId] {
        assert!(!b.record_outcome(o));
    }
    assert!(b.record_outcome(Outcome::WrongId), "the 3rd wrong id opens");
    assert!(b.is_open());
    assert!(!Outcome::Answered.is_silence() && !Outcome::Refused.is_silence());
    assert!(Outcome::Timeout.is_silence() && Outcome::WrongId.is_silence());
    // Linux classification: a fast EIO is the keyboard's refusal ...
    let g = ms(4_500);
    let eio = || -> io::Result<Vec<u8>> { Err(io::Error::from_raw_os_error(libc::EIO)) };
    assert_eq!(Outcome::classify(&eio(), 0x47, ms(5), g), Outcome::Refused);
    assert_eq!(Outcome::classify(&eio(), 0x47, ms(999), g), Outcome::Refused);
    // ... a slow one (BlueZ HIDP timeout, 3.4-3.6 s) is a silence
    assert_eq!(Outcome::classify(&eio(), 0x47, ms(1000), g), Outcome::Timeout);
    assert_eq!(Outcome::classify(&eio(), 0x47, ms(3_400), g), Outcome::Timeout);
    // explicit timeouts and lost links are silences however fast
    let kinds = [io::ErrorKind::TimedOut, io::ErrorKind::NotConnected, io::ErrorKind::BrokenPipe, io::ErrorKind::ConnectionAborted, io::ErrorKind::ConnectionReset];
    for k in kinds {
        assert_eq!(Outcome::classify(&Err(io::Error::from(k)), 0x47, ms(1), g), Outcome::Timeout, "{k:?}");
    }
    for e in [libc::ENODEV, libc::ENXIO, libc::EBADF, libc::ETIMEDOUT, libc::EHOSTDOWN, libc::ENOTCONN, libc::ESHUTDOWN] {
        let r: io::Result<Vec<u8>> = Err(io::Error::from_raw_os_error(e));
        assert_eq!(Outcome::classify(&r, 0x47, ms(1), g), Outcome::Timeout, "{e}");
    }
    let nf: io::Result<Vec<u8>> = Err(io::Error::from(io::ErrorKind::NotFound));
    assert_eq!(Outcome::classify(&nf, 0x47, ms(1), g), Outcome::Refused);
    // DATA of another id, or empty: ignored, the request expires
    assert_eq!(Outcome::classify(&Ok(vec![0x46, 1]), 0x47, ms(5), g), Outcome::WrongId);
    assert_eq!(Outcome::classify(&Ok(vec![]), 0x47, ms(5), g), Outcome::WrongId);
}

// ── R4 ─────────────────────────────────────────────────────────────────────

/// R4 — [décompilé] `handleSleep` `0xfffffe000a35abe4`: flag 0, counter 1,
/// `_mUseSleepTimeout` (4500 / 5500 ms for the first request, then back);
/// `IOAppleBluetoothHIDDriver::handleSleep` `0xfffffe000a354d5c` stops the
/// battery timer, `handleWake` `0xfffffe000a354e40` restarts it (60 s).
#[test]
fn r4_after_sleep_two_silences_suffice() {
    let t0 = Instant::now();
    let mut m = ready(t0);
    m.sleep();
    assert!(m.link.is_sleeping());
    assert_eq!(m.link.next_battery(), None, "stopBatteryUpdate");
    assert_eq!(m.breaker.counter(), 1);
    assert_eq!(m.send(GET47, t0 + s(10)), Err(Refusal::Sleeping));
    let tw = t0 + s(3600);
    m.wake(tw);
    assert_eq!(m.link.next_battery(), Some(tw + s(60)), "startBatteryUpdate");
    let t = m.send(GET47, tw + s(60)).unwrap();
    assert_eq!(t, Timeouts { wait: ms(4_500), guard: ms(5_500) });
    m.poll(tw + s(60) + ms(5_499));
    assert_eq!(m.in_flight(), Some(GET47));
    m.poll(tw + s(60) + ms(5_500));
    assert_eq!(m.breaker.counter(), 2);
    // the long timeouts were for the first request only
    let t = m.send(GET47, tw + s(70)).unwrap();
    assert_eq!(t.wait, ms(3_500));
    m.complete(Outcome::Timeout, tw + s(74));
    assert!(m.breaker.is_open(), "2 silences after a sleep");
    assert_eq!(m.take_effects(), vec![Effect::RequestDisconnect]);
    // a sleep closes an open breaker (counter 1 again)...
    m.sleep();
    assert!(!m.breaker.is_open() && m.breaker.counter() == 1);
    // ...but the same connection never asks twice for the disconnection
    m.wake(tw + s(100));
    exchange(&mut m, GET47, Outcome::Timeout, tw + s(200)).unwrap();
    exchange(&mut m, GET47, Outcome::Timeout, tw + s(210)).unwrap();
    assert!(m.breaker.is_open());
    assert!(m.take_effects().is_empty());
    // a wake on a device that is not ready schedules nothing
    let mut l = LinkModel::new();
    l.connected();
    l.sleep();
    l.wake(t0);
    assert_eq!(l.next_battery(), None);
    // SET_PROTOCOL answered while asleep: ready, timer from the wake only
    l.sleep();
    l.protocol_result(true, t0);
    assert!(l.is_ready() && l.next_battery().is_none());
    l.sleep();
    l.wake(t0 + s(5));
    assert_eq!(l.next_battery(), Some(t0 + s(65)));
    // sleep during a read: the read is dropped
    assert!(l.begin_cycle(t0 + s(65)));
    l.sleep();
    assert!(!l.cycle_running());
}

// ── R5 ─────────────────────────────────────────────────────────────────────

/// R5 — [décompilé] `updateBatteryLevel` `0xfffffe000a35611c` (>= 100 -> 100),
/// `batteryPercent` `0x19641ba94` (54/21/2.4375), `updateBatteryState`
/// `0xfffffe000a356378` (notice on a change only, > 3 refused),
/// `serviceInterestOfType` `0x19641b318` (`rbsn`/`btlw`/`btpn`).
#[test]
fn r5_percent_display_and_state_notices() {
    assert_eq!(raw_percent(0), 0);
    assert_eq!(raw_percent(99), 99);
    assert_eq!(raw_percent(100), 100);
    assert_eq!(raw_percent(101), 100);
    assert_eq!(raw_percent(255), 100);
    assert_eq!(display_percent(20), 20.0);
    assert_eq!(display_percent(21), 21.0);
    assert_eq!(display_percent(22), 23.4375);
    assert_eq!(display_percent(53), 21.0 + 32.0 * 2.4375);
    assert_eq!(display_percent(54), 100.0);
    assert_eq!(display_percent(100), 100.0);
    assert_eq!(display_percent(101), 101.0, "above 100: unchanged in batteryPercent");
    // same curve as the register map's (used for the published figure)
    for b in 0..=255u8 {
        assert_eq!(display_percent(raw_percent(b)), crate::registry::apple_display_percent(b), "{b}");
    }
    // battery state
    assert_eq!(BatteryNotice::of_state(0), Some(BatteryNotice::Normal));
    assert_eq!(BatteryNotice::of_state(1), Some(BatteryNotice::Low));
    assert_eq!(BatteryNotice::of_state(2), Some(BatteryNotice::DangerouslyLow));
    assert_eq!(BatteryNotice::of_state(3), Some(BatteryNotice::DangerouslyLow));
    assert_eq!(BatteryNotice::of_state(4), None);
    let mut m = AppleModel::new(true);
    m.connected();
    assert_eq!(m.battery_state(0), None, "state 0 memorised at deviceReady");
    assert_eq!(m.battery_state(1), Some(BatteryNotice::Low));
    assert_eq!(m.battery_state(1), None, "no change, no notice");
    assert_eq!(m.battery_state(9), None, "invalid");
    assert_eq!(m.battery_state(3), Some(BatteryNotice::DangerouslyLow));
    assert_eq!(m.battery_state(2), Some(BatteryNotice::DangerouslyLow), "2 and 3 are both btpn");
    assert_eq!(m.battery_state(1), Some(BatteryNotice::Low));
    assert_eq!(m.battery_state(0), Some(BatteryNotice::Normal));
    assert_eq!(
        m.take_effects(),
        [BatteryNotice::Low, BatteryNotice::DangerouslyLow, BatteryNotice::DangerouslyLow, BatteryNotice::Low, BatteryNotice::Normal]
            .map(Effect::Battery)
            .to_vec()
    );
    // pushed A1 30 xx
    m.input(&[0x30, 2]);
    assert_eq!(m.take_effects(), vec![Effect::Battery(BatteryNotice::DangerouslyLow)]);
    m.input(&[0x30, 2, 0]);
    m.input(&[0x30]);
    assert!(m.take_effects().is_empty(), "3 bytes on the wire exactly");
    // a new connection forgets the state
    m.connected();
    assert_eq!(m.battery_state(2), Some(BatteryNotice::DangerouslyLow));
    assert_eq!(m.percent(), None);
    assert_eq!(m.battery_percent(150), 100);
    assert_eq!(m.percent(), Some(100));
    // the project's alert of the pushed state agrees on every rise
    for old in 0..=3u8 {
        for new in 0..=3u8 {
            if let Some(a) = crate::passive::battery_state_alert(Some(old), new) {
                let want = if a == crate::registry::BatteryState::Low { BatteryNotice::Low } else { BatteryNotice::DangerouslyLow };
                assert_eq!(BatteryNotice::of_state(new), Some(want));
            }
        }
    }
}

// ── R6 ─────────────────────────────────────────────────────────────────────

/// R6 — [décompilé] `AppleBluetoothHIDKeyboard::processInterruptData`
/// `0xfffffe00090758a0`: `A1 13 xx` with bit 1 = 0 -> `KeyboardOff` and the
/// next disconnection is not announced (`+0x169`).
#[test]
fn r6_keyboard_off_silences_the_disconnection() {
    for b in 0..=255u8 {
        let want = (b & 0x02 == 0).then_some(InputEvent::KeyboardOff);
        assert_eq!(decode_input(&[0x13, b]), want, "{b:#04x}");
        // same verdict as the passive listener
        let passive = crate::passive::decode(&[0x13, b]) == Some(crate::passive::PassiveEvent::KeyboardOff);
        assert_eq!(passive, want.is_some());
    }
    assert_eq!(decode_input(&[0x13, 0, 0]), None);
    assert_eq!(decode_input(&[0x13]), None);
    assert_eq!(decode_input(&[0x30, 1]), Some(InputEvent::BatteryState(1)));
    assert_eq!(decode_input(&[0x12, 0]), None);
    assert_eq!(decode_input(&[]), None);
    let mut m = AppleModel::new(true);
    m.connected();
    assert_eq!(m.disconnected(), Disconnection::Announced);
    m.connected();
    m.input(&[0x13, 0x03]);
    assert!(m.take_effects().is_empty());
    m.input(&[0x13, 0x01]);
    assert_eq!(m.take_effects(), vec![Effect::KeyboardOff]);
    assert_eq!(m.disconnected(), Disconnection::Silent);
    assert_eq!(m.disconnected(), Disconnection::Announced, "once");
    m.input(&[0x13, 0x00]);
    m.connected();
    assert_eq!(m.disconnected(), Disconnection::Announced, "forgotten at reconnection");
}

// ── R7 ─────────────────────────────────────────────────────────────────────

/// R7 — [décompilé] `willShutdown` `0xfffffe000a35674c` ->
/// `setExtendedReport("WillShutdown")` `0xfffffe000a356f84`: SET Feature
/// `0x40`, id alone, through the same breaker and spacing.
#[test]
fn r7_will_shutdown_once_and_only_when_allowed() {
    let t0 = Instant::now();
    let mut m = AppleModel::new(true);
    assert_eq!(m.will_shutdown(t0), Err(Refusal::NotConnected));
    let mut m = ready(t0);
    assert_eq!(m.will_shutdown(t0 + ms(500)), Err(Refusal::TooSoon(ms(500))));
    assert!(m.will_shutdown(t0 + s(1)).is_ok());
    assert_eq!(m.in_flight(), Some(Request::SetFeature(0x40)));
    m.complete(Outcome::Answered, t0 + s(1));
    assert_eq!(m.will_shutdown(t0 + s(10)), Err(Refusal::AlreadySent));
    // the project's writer applies the same rules (parity.rs)
    use crate::parity::{will_shutdown, FeatureSink, Outcome as P};
    use crate::registry::WriteSession;
    use std::sync::Mutex;
    struct Sink(std::cell::RefCell<Vec<Vec<u8>>>);
    impl FeatureSink for Sink {
        fn set_feature(&self, _op: crate::registry::WriteOp, r: &[u8]) -> io::Result<()> {
            self.0.borrow_mut().push(r.to_vec());
            Ok(())
        }
    }
    let sink = Sink(Default::default());
    let (mut ws, b) = (WriteSession::new(), Mutex::new(Breaker::new()));
    let mut nap = |_: Duration| {};
    assert_eq!(will_shutdown(true, true, &sink, &mut ws, &b, None, &mut nap), P::Sent);
    assert_eq!(will_shutdown(true, true, &sink, &mut ws, &b, None, &mut nap), P::AlreadySent);
    assert_eq!(*sink.0.borrow(), vec![vec![0x40]], "the id alone, once");
    let mut ws = WriteSession::new();
    for _ in 0..APPLE.trip_after {
        b.lock().unwrap().record(false);
    }
    assert_eq!(will_shutdown(true, true, &sink, &mut ws, &b, None, &mut nap), P::BreakerOpen);
}

// ── scenarios ──────────────────────────────────────────────────────────────

/// Connection -> 60 s -> read (0x47, 0x30) -> 4 h -> read.
#[test]
fn scenario_connection_then_60s_then_4h() {
    let t0 = Instant::now();
    let mut m = AppleModel::new(true);
    m.connected();
    exchange(&mut m, Request::SetProtocol, Outcome::Answered, t0).unwrap();
    let mut reads = Vec::new();
    let mut t = t0;
    let end = t0 + s(9 * 3600);
    while t < end {
        if m.link.begin_cycle(t) {
            reads.push(t);
            let mut tt = t;
            for req in BATTERY_READ {
                exchange(&mut m, req, Outcome::Answered, tt).unwrap();
                tt += s(1);
            }
            m.link.end_cycle(true, true, t);
        }
        t = m.next_deadline().unwrap_or(end).max(t + s(1));
    }
    assert_eq!(reads, vec![t0 + s(60), t0 + s(60 + 4 * 3600), t0 + s(60 + 8 * 3600)]);
}

/// 3 timeouts -> nothing more goes out + one disconnection request; the
/// reconnection reopens the breaker.
#[test]
fn scenario_three_timeouts_then_reconnection() {
    let t0 = Instant::now();
    let mut m = ready(t0);
    let mut emitted = 0;
    let mut t = t0 + s(60);
    for _ in 0..20 {
        if m.send(GET47, t).is_ok() {
            emitted += 1;
            m.poll(t + s(5));
        }
        t += s(10);
    }
    assert_eq!(emitted, 3);
    assert_eq!(m.take_effects(), vec![Effect::RequestDisconnect]);
    assert_eq!(m.disconnected(), Disconnection::Announced);
    m.connected();
    assert!(!m.breaker.is_open() && m.breaker.counter() == 0);
    exchange(&mut m, Request::SetProtocol, Outcome::Answered, t).unwrap();
    assert!(exchange(&mut m, GET47, Outcome::Answered, t + s(1)).is_ok());
    // and a new episode may ask again
    for i in 2..5 {
        exchange(&mut m, GET47, Outcome::Timeout, t + s(i)).unwrap();
    }
    assert_eq!(m.take_effects(), vec![Effect::RequestDisconnect]);
}

/// A 0x03 refusal between two silences resets the count: no trip.
#[test]
fn scenario_refusal_resets_the_counter() {
    let t0 = Instant::now();
    let mut m = ready(t0);
    let outs = [Outcome::Timeout, Outcome::Timeout, Outcome::Refused, Outcome::Timeout, Outcome::Timeout];
    for (i, o) in outs.into_iter().enumerate() {
        exchange(&mut m, GET47, o, t0 + s(i as u64 + 1)).unwrap();
    }
    assert!(!m.breaker.is_open());
    assert_eq!(m.breaker.counter(), 2);
    assert!(m.take_effects().is_empty());
}

#[test]
fn next_deadline_is_the_earliest_of_watchdog_and_timer() {
    let t0 = Instant::now();
    let mut m = AppleModel::new(true);
    assert_eq!(m.next_deadline(), None);
    m.connected();
    m.send(Request::SetProtocol, t0).unwrap();
    assert_eq!(m.next_deadline(), Some(t0 + s(11)));
    m.complete(Outcome::Answered, t0);
    assert_eq!(m.next_deadline(), Some(t0 + s(60)));
    m.send(GET47, t0 + s(58)).unwrap();
    assert_eq!(m.next_deadline(), Some(t0 + s(60)), "timer first");
    m.complete(Outcome::Answered, t0 + s(58));
    m.send(GET47, t0 + s(59)).unwrap();
    assert_eq!(m.next_deadline(), Some(t0 + s(60)));
    let mut late = ready(t0);
    late.link.begin_cycle(t0 + s(60));
    late.send(GET47, t0 + s(60)).unwrap();
    assert_eq!(late.next_deadline(), Some(t0 + s(60) + ms(4_500)), "watchdog only");
}

#[test]
fn invariants_catch_impossible_states() {
    let mut b = Breaker::new();
    assert!(b.check().is_ok());
    b.open = true;
    assert!(b.check().is_err());
    let mut b = Breaker::new();
    b.disconnect_pending = true;
    assert!(b.check().is_err());
    let mut b = Breaker::new();
    b.probe = true;
    assert!(b.check().is_err());
    b.alive();
    assert!(!b.probe, "alive on a closed breaker arms nothing");
    let t0 = Instant::now();
    let mut l = LinkModel::new();
    l.next_battery = Some(t0);
    assert!(l.check().is_err());
    l.connected();
    l.protocol_result(true, t0);
    assert!(l.check().is_ok());
    l.sleeping = true;
    assert!(l.check().is_err());
    let mut l = LinkModel::new();
    l.cycle = true;
    assert!(l.check().is_err());
    let mut m = ready(t0);
    assert!(m.check().is_ok());
    m.send(GET47, t0 + s(1)).unwrap();
    m.link.sleeping = true;
    m.link.next_battery = None;
    assert!(m.link.check().is_ok());
    assert!(m.check().is_err());
    let mut m = ready(t0);
    m.send(GET47, t0 + s(1)).unwrap();
    m.link.state = LinkState::Disconnected;
    m.link.next_battery = None;
    assert!(m.link.check().is_ok());
    assert!(m.check().is_err());
    let mut m = ready(t0);
    m.battery_state = 4;
    assert!(m.check().is_err());
    let mut m = ready(t0);
    m.link.cycle = true;
    m.link.state = LinkState::NotReady;
    assert!(m.check().is_err());
    // a sleep drops the request in flight
    let mut m = ready(t0);
    m.send(GET47, t0 + s(1)).unwrap();
    m.sleep();
    assert_eq!(m.in_flight(), None);
    assert!(m.check().is_ok());
}

// ── properties ─────────────────────────────────────────────────────────────

mod props {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        Connect,
        Disconnect,
        Sleep,
        Wake,
        Advance(u64),
        Send(u8),
        Complete(u8),
        Poll,
        Input(u8, u8),
        Battery(bool),
        Shutdown,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            1 => Just(Op::Connect),
            1 => Just(Op::Disconnect),
            1 => Just(Op::Sleep),
            1 => Just(Op::Wake),
            4 => (0u64..6_000).prop_map(Op::Advance),
            5 => (0u8..6).prop_map(Op::Send),
            4 => (0u8..4).prop_map(Op::Complete),
            2 => Just(Op::Poll),
            1 => (prop_oneof![Just(0x13u8), Just(0x30), any::<u8>()], any::<u8>()).prop_map(|(a, b)| Op::Input(a, b)),
            2 => any::<bool>().prop_map(Op::Battery),
            1 => Just(Op::Shutdown),
        ]
    }

    fn req(k: u8) -> Request {
        match k {
            0 => GET47,
            1 => Request::GetInput(0x30),
            2 => Request::SetFeature(0x40),
            3 => Request::SetProtocol,
            4 => Request::Output,
            _ => Request::HidControl(3),
        }
    }

    fn outcome(k: u8) -> Outcome {
        [Outcome::Answered, Outcome::Refused, Outcome::Timeout, Outcome::WrongId][k as usize % 4]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Never an emission while the breaker is open, never two requests
        /// less than 1 s apart, never more than one disconnection request per
        /// connection, never an impossible state.
        #[test]
        fn apple_model_properties(disconnect in any::<bool>(), ops in proptest::collection::vec(op(), 1..120)) {
            let t0 = Instant::now();
            let mut t = t0;
            let mut m = AppleModel::new(disconnect);
            let mut last_sent: Option<Instant> = None;
            let mut asks_this_episode = 0;
            for o in ops {
                match o {
                    Op::Connect => {
                        m.connected();
                        asks_this_episode = 0;
                    }
                    Op::Disconnect => {
                        m.disconnected();
                    }
                    Op::Sleep => m.sleep(),
                    Op::Wake => m.wake(t),
                    Op::Advance(d) => t += Duration::from_millis(d),
                    Op::Send(k) => {
                        let open = !m.breaker.would_allow();
                        if m.send(req(k), t).is_ok() {
                            prop_assert!(!open, "emission while open");
                            if let Some(l) = last_sent {
                                prop_assert!(t.duration_since(l) >= APPLE.min_gap, "two requests < 1 s");
                            }
                            prop_assert!(m.link.state() != LinkState::NotReady);
                            last_sent = Some(t);
                        }
                    }
                    Op::Complete(k) => m.complete(outcome(k), t),
                    Op::Poll => m.poll(t),
                    Op::Input(a, b) => m.input(&[a, b]),
                    Op::Battery(ok) => {
                        if m.link.begin_cycle(t) {
                            m.link.end_cycle(true, ok, t);
                        }
                    }
                    Op::Shutdown => {
                        let open = !m.breaker.would_allow();
                        if m.will_shutdown(t).is_ok() {
                            prop_assert!(!open);
                            if let Some(l) = last_sent {
                                prop_assert!(t.duration_since(l) >= APPLE.min_gap);
                            }
                            last_sent = Some(t);
                        }
                    }
                }
                let asks = m.take_effects().iter().filter(|e| **e == Effect::RequestDisconnect).count();
                prop_assert!(disconnect || asks == 0, "disabled: never asked");
                asks_this_episode += asks;
                prop_assert!(asks_this_episode <= 1, "more than one disconnection request");
                if let Err(e) = m.check() {
                    prop_assert!(false, "impossible state: {e}");
                }
            }
        }

        /// The breaker alone, whatever the outcomes: opens exactly at the
        /// 3rd consecutive silence, and an answer always closes it.
        #[test]
        fn breaker_counts_consecutive_silences(outs in proptest::collection::vec(0u8..4, 0..60)) {
            let mut b = Breaker::new();
            let mut run = 0u32;
            for k in outs {
                let o = outcome(k);
                b.record_outcome(o);
                run = if o.is_silence() { run + 1 } else { 0 };
                prop_assert_eq!(b.is_open(), run >= APPLE.trip_after);
                prop_assert_eq!(b.counter(), run);
                prop_assert!(b.check().is_ok());
            }
        }
    }
}
