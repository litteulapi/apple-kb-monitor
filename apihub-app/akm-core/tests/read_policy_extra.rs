//! Targeted tests of the safe read policy.

use akm_core::decode::HidSource;
use akm_core::read_policy::{
    build_report_safe, gate, is_allowed, last_input_age, lock_path, note_input, read_safe,
    try_lock, Gate, SafeRead, SafeSource, ACTIVE_WINDOW, ALLOWED, BUDGET, LOCK_WAIT, MIN_GAP,
    TRIP_AFTER,
};
use akm_core::registry;
use akm_core::report::{KbReport, KbWake};
use std::cell::RefCell;
use std::io;
use std::os::fd::AsRawFd;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn private_runtime_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("akm-rp-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &d);
    d
}

struct Src {
    calls: RefCell<Vec<(u8, Instant)>>,
    delay: Duration,
    answers: Vec<(u8, Vec<u8>)>,
    inputs: Vec<(u8, Vec<u8>)>,
}
impl Src {
    fn new(delay_ms: u64, answers: Vec<(u8, Vec<u8>)>) -> Self {
        Self {
            calls: RefCell::new(vec![]),
            delay: Duration::from_millis(delay_ms),
            answers,
            inputs: vec![(0x30, vec![0x30, 0])],
        }
    }
    fn ids(&self) -> Vec<u8> {
        self.calls.borrow().iter().map(|c| c.0).collect()
    }
    fn answer(&self, table: &[(u8, Vec<u8>)], id: u8) -> io::Result<Vec<u8>> {
        self.calls.borrow_mut().push((id, Instant::now()));
        std::thread::sleep(self.delay);
        table
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}
impl HidSource for Src {
    fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
        self.answer(&self.answers, id)
    }
    fn input(&self, id: u8) -> io::Result<Vec<u8>> {
        self.answer(&self.inputs, id)
    }
}

const BCM: &str = "HID_ID=0005:000005AC:00000256\nHID_NAME=Kb\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n";

#[test]
fn constants_are_the_documented_policy() {
    assert_eq!(ALLOWED, [0x47, 0x46, 0x49]);
    assert_eq!(ACTIVE_WINDOW, Duration::from_mins(1));
    // Apple spaces its requests by 1 s and stops after 3 timeouts: never below that.
    assert_eq!(MIN_GAP, Duration::from_secs(1));
    assert_eq!(TRIP_AFTER, 3);
    assert_eq!(
        akm_core::read_policy::SLOW_ANSWER,
        Duration::from_millis(1500)
    );
    assert_eq!(
        BUDGET,
        (MIN_GAP + akm_core::read_policy::SLOW_ANSWER) * 4,
        "4 routine requests (0x47, Input 0x30, 0x46, 0x49), each 1 s after a slow answer"
    );
    assert_eq!(
        akm_core::read_policy::ONCE_BUDGET,
        (MIN_GAP + akm_core::read_policy::SLOW_ANSWER) * 6,
        "0x4F, 0x60 and the four name fragments in ONE burst"
    );
    assert_eq!(LOCK_WAIT, Duration::from_millis(500));
    assert_eq!(registry::SAFE_READ_IDS, [0x47, 0x46, 0x49]);
    assert_eq!(registry::SAFE_READ_INPUT_IDS, [0x30]);
    assert_eq!(
        akm_core::read_policy::routine_reads(),
        vec![
            akm_core::apple_model::Request::GetFeature(0x47),
            akm_core::apple_model::Request::GetInput(0x30),
            akm_core::apple_model::Request::GetFeature(0x46),
            akm_core::apple_model::Request::GetFeature(0x49),
        ]
    );
    assert_eq!(
        &akm_core::read_policy::routine_reads()[..2],
        &akm_core::apple_model::BATTERY_READ[..]
    );
    assert_eq!(registry::DAEMON_ONCE_IDS, [0x4F, 0x60]);
    for id in 0..=255u8 {
        let expected = matches!(id, 0x47 | 0x46 | 0x49 | 0x4F | 0x51..=0x54 | 0x60);
        assert_eq!(is_allowed(id), expected, "{id:#x}");
    }
    for id in [0xFEu8, 0x4C, 0x01] {
        assert!(!is_allowed(id), "{id:#x} must stay refused");
    }
}

#[test]
fn gate_boundaries() {
    assert_eq!(gate(None), Gate::Idle);
    assert_eq!(gate(Some(Duration::ZERO)), Gate::Allowed);
    assert_eq!(
        gate(Some(
            ACTIVE_WINDOW.checked_sub(Duration::from_millis(1)).unwrap()
        )),
        Gate::Allowed
    );
    assert_eq!(gate(Some(ACTIVE_WINDOW)), Gate::Idle);
    assert_eq!(
        gate(Some(ACTIVE_WINDOW + Duration::from_millis(1))),
        Gate::Idle
    );
    assert_eq!(gate(Some(Duration::from_hours(1))), Gate::Idle);
}

#[test]
fn input_age_is_measured_from_the_last_note() {
    let _g = serial();
    note_input();
    let a = last_input_age(Instant::now()).unwrap();
    assert!(a < Duration::from_secs(1));
    let b = last_input_age(Instant::now() + Duration::from_secs(30)).unwrap();
    assert!(
        b >= Duration::from_secs(30) && b < Duration::from_secs(31),
        "{b:?}"
    );
}

#[test]
fn lock_path_follows_xdg_runtime_dir() {
    let _g = serial();
    let d = private_runtime_dir("path");
    assert_eq!(lock_path(), d.join("apple-kb-monitor").join("hid.lock"));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn lock_is_exclusive_in_process_and_released_on_drop() {
    let _g = serial();
    let d = private_runtime_dir("excl");
    let first = try_lock(Duration::ZERO).expect("lock free");
    assert!(lock_path().is_file(), "the lock file is created");
    assert!(
        try_lock(Duration::ZERO).is_none(),
        "second reader of the same process refused"
    );
    drop(first);
    let again = try_lock(Duration::ZERO);
    assert!(again.is_some(), "lock released after drop");
    drop(again);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn lock_waits_then_gives_up_on_a_foreign_flock() {
    let _g = serial();
    let d = private_runtime_dir("flock");
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(d.join("apple-kb-monitor"))
            .unwrap();
    }
    let other = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path())
        .unwrap();
    // SAFETY: valid fd owned by `other`.
    assert_eq!(
        unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let t = Instant::now();
    assert!(try_lock(Duration::from_millis(300)).is_none());
    let e = t.elapsed();
    assert!(e >= Duration::from_millis(290), "gave up too early: {e:?}");
    assert!(e < Duration::from_millis(900), "waited too long: {e:?}");
    let t = Instant::now();
    assert!(try_lock(Duration::ZERO).is_none());
    assert!(t.elapsed() < Duration::from_millis(40));
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        drop(other);
    });
    assert!(try_lock(Duration::from_millis(1500)).is_some());
    h.join().unwrap();
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn safe_source_spacing_is_enforced_but_not_added_when_already_late() {
    let src = Src::new(0, vec![(0x47, vec![0x47, 1]), (0x46, vec![0x46, 0, 0])]);
    let s = SafeSource::new(&src);
    s.feature(0x47).unwrap();
    s.feature(0x46).unwrap();
    let c = src.calls.borrow();
    let gap = c[1].1.duration_since(c[0].1);
    assert!(
        gap >= MIN_GAP.checked_sub(Duration::from_millis(5)).unwrap(),
        "{gap:?}"
    );
    assert!(gap < MIN_GAP + Duration::from_millis(150), "{gap:?}");
    drop(c);
    let before = src.calls.borrow().len();
    s.feature(0x47).unwrap();
    // Bounds sit halfway between the right wait and the wrong one, not at scheduler precision.
    let late = Duration::from_millis(400);
    std::thread::sleep(late);
    s.feature(0x46).unwrap();
    {
        let c = src.calls.borrow();
        let gap = c[before + 1].1.duration_since(c[before].1);
        assert!(
            gap >= MIN_GAP.checked_sub(Duration::from_millis(5)).unwrap(),
            "{gap:?}"
        );
        assert!(
            gap < MIN_GAP + late / 2,
            "the gap counts from the last call: {gap:?}"
        );
    }
    std::thread::sleep(MIN_GAP + Duration::from_millis(100));
    let t = Instant::now();
    s.feature(0x47).unwrap();
    assert!(t.elapsed() < MIN_GAP / 2, "{:?}", t.elapsed());
    assert_eq!(s.sent(), 5);
}

#[test]
fn refused_ids_do_not_count_nor_delay() {
    let src = Src::new(0, vec![]);
    let s = SafeSource::new(&src);
    let t = Instant::now();
    for id in [0u8, 0x45, 0x48, 0x4A, 0xEA, 0xFE] {
        assert_eq!(
            s.feature(id).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    assert!(t.elapsed() < Duration::from_millis(40));
    assert_eq!(s.sent(), 0);
    let got = src.ids();
    assert!(got.is_empty(), "{got:?}");
}

fn full() -> Vec<(u8, Vec<u8>)> {
    vec![
        (0x47, vec![0x47, 80]),
        (0x46, vec![0x46, 0xA0, 0x0B]),
        (0x49, vec![0x49, 0x89, 0x0B]),
    ]
}

#[test]
fn read_safe_stores_raw_hex_and_decodes() {
    let src = Src::new(0, full());
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert_eq!(src.ids(), vec![0x47, 0x30, 0x46, 0x49]);
    assert_eq!(r.raw.get("0x47").map(String::as_str), Some("50"));
    assert_eq!(r.raw.get("0x46").map(String::as_str), Some("a00b"));
    assert_eq!(r.raw.get("0x49").map(String::as_str), Some("890b"));
    assert_eq!(r.battery.percentage, Some(80.0));
    assert_eq!(r.battery.voltage, Some(2.976));
    assert!(!r.incomplete);
}

#[test]
fn read_safe_percentage_bounds_and_no_overwrite() {
    for (p, ok) in [(0u8, true), (100, true), (101, false), (255, false)] {
        let src = Src::new(
            0,
            vec![
                (0x47, vec![0x47, p]),
                (0x46, vec![0x46, 0, 0]),
                (0x49, vec![0x49, 0, 0]),
            ],
        );
        let mut r = KbReport::default();
        read_safe(&src, &mut r);
        assert_eq!(r.battery.percentage, ok.then_some(f64::from(p)), "pct {p}");
    }
    let src = Src::new(0, full());
    let mut r = KbReport::default();
    r.battery.percentage = Some(42.0);
    read_safe(&src, &mut r);
    assert_eq!(r.battery.percentage, Some(42.0));
}

#[test]
fn read_safe_voltage_bounds() {
    for (mv, ok) in [
        (1499u16, false),
        (1500, true),
        (2976, true),
        (3700, true),
        (3701, false),
        (0, false),
        (65535, false),
    ] {
        let le = mv.to_le_bytes();
        let src = Src::new(
            0,
            vec![
                (0x47, vec![0x47, 50]),
                (0x46, vec![0x46, le[0], le[1]]),
                (0x49, vec![0x49, 0, 0]),
            ],
        );
        let mut r = KbReport::default();
        read_safe(&src, &mut r);
        assert_eq!(
            r.battery.voltage,
            ok.then(|| f64::from(mv) / 1000.0),
            "{mv} mV"
        );
    }
}

#[test]
fn read_safe_short_answers() {
    let src = Src::new(
        0,
        vec![
            (0x47, vec![0x47, 70]),
            (0x46, vec![0x46, 0xA0]),
            (0x49, vec![0x49, 1, 2]),
        ],
    );
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert_eq!(r.battery.voltage, None);
    assert_eq!(r.raw.get("0x46").map(String::as_str), Some("a0"));
    assert_eq!(src.ids(), vec![0x47, 0x30, 0x46, 0x49]);
    let src = Src::new(
        0,
        vec![
            (0x47, vec![0x47]),
            (0x46, vec![0x46, 0xA0, 0x0B]),
            (0x49, vec![0x49]),
        ],
    );
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert!(!r.raw.contains_key("0x47") && !r.raw.contains_key("0x49"));
    assert_eq!(r.battery.percentage, None);
    assert_eq!(r.battery.voltage, Some(2.976));
    assert_eq!(src.ids(), vec![0x47, 0x30, 0x46, 0x49]);
}

#[test]
fn read_safe_stops_at_first_failure_and_flags_incomplete() {
    for missing in [0x47u8, 0x46, 0x49] {
        let ans: Vec<_> = full().into_iter().filter(|(i, _)| *i != missing).collect();
        let src = Src::new(0, ans);
        let mut r = KbReport::default();
        assert_eq!(
            read_safe(&src, &mut r),
            SafeRead::Partial,
            "missing {missing:#x}"
        );
        assert!(r.incomplete);
        let n = [0x47u8, 0x30, 0x46, 0x49]
            .iter()
            .position(|i| *i == missing)
            .unwrap();
        assert_eq!(src.ids().len(), n + 1, "stop at the first failure");
    }
}

#[test]
fn read_safe_stops_when_the_budget_is_spent() {
    let src = Src::new(3000, full());
    let mut r = KbReport::default();
    let t = Instant::now();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Partial);
    assert_eq!(src.ids(), vec![0x47, 0x30, 0x46]);
    assert!(r.incomplete);
    assert!(t.elapsed() < Duration::from_millis(11_800));
}

#[test]
fn read_safe_is_complete_when_every_answer_takes_one_second() {
    let src = Src::new(1100, full());
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert_eq!(src.ids(), vec![0x47, 0x30, 0x46, 0x49]);
    assert!(!r.incomplete);
}

#[test]
fn build_report_safe_paths() {
    let _g = serial();
    let d = private_runtime_dir("build");
    let wake = KbWake {
        last_age_s: Some(1.5),
        count: 7,
    };

    // Unknown family: never any I/O, policy "Allowed" without a read.
    let src = Src::new(0, full());
    let (r, o) = build_report_safe("", None, &src, wake.clone(), Instant::now());
    assert_eq!(o, SafeRead::Skipped(Gate::Allowed));
    let got = src.ids();
    assert!(got.is_empty(), "{got:?}");
    assert!(r.bluetooth.connected);
    assert_eq!(r.wake, wake);

    note_input();
    let mut ans = full();
    ans.push((0x4F, vec![0x4F, 1, 2, 3, 4]));
    ans.push((0x60, vec![0x60, 5, 6]));
    for id in [0x51u8, 0x52, 0x53, 0x54] {
        ans.push((id, vec![id, b'a', 0, 0, 0, 0, 0, 0, 0]));
    }
    let src = Src::new(0, ans);
    let (r, o) = build_report_safe(BCM, None, &src, wake.clone(), Instant::now());
    assert_eq!(o, SafeRead::Complete);
    let ids = src.ids();
    assert_eq!(ids[..6], [0x47, 0x30, 0x46, 0x49, 0x4F, 0x60]);
    assert!(
        [0x51u8, 0x52, 0x53, 0x54].starts_with(&ids[6..]),
        "{ids:x?}"
    );
    assert_eq!(r.battery.percentage, Some(80.0));
    assert_eq!(r.battery.percentage_fine, Some(80.0));
    assert_eq!(r.wake, wake);
    assert!(r.bluetooth.connected);

    let src = Src::new(0, full());
    let (_, o) = build_report_safe(
        BCM,
        None,
        &src,
        wake.clone(),
        Instant::now() + ACTIVE_WINDOW * 2,
    );
    assert_eq!(o, SafeRead::Skipped(Gate::Idle));
    let got = src.ids();
    assert!(got.is_empty(), "{got:?}");

    let held = try_lock(Duration::ZERO).unwrap();
    let src = Src::new(0, full());
    let (r, o) = build_report_safe(BCM, None, &src, wake, Instant::now());
    assert_eq!(o, SafeRead::Skipped(Gate::Busy));
    let got = src.ids();
    assert!(got.is_empty(), "{got:?}");
    assert_eq!(r.battery.percentage_fine, r.battery.percentage);
    drop(held);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn name_only_schedule_reads_the_four_fragments_and_nothing_else() {
    let _g = serial();
    let d = private_runtime_dir("nameonly");
    akm_core::read_policy::note_connection();
    let mut ans = full();
    ans.push((0x4F, vec![0x4F, 1, 2, 3, 4]));
    ans.push((0x60, vec![0x60, 5, 6]));
    for id in [0x51u8, 0x52, 0x53, 0x54] {
        ans.push((id, vec![id, b'N', b'o', b'm', 0, 0, 0, 0, 0]));
    }
    let src = Src::new(0, ans.clone());
    akm_core::read_policy::set_name_only_schedule();
    let (r, o) = build_report_safe(BCM, None, &src, KbWake::default(), Instant::now());
    assert_eq!(o, SafeRead::Complete);
    assert_eq!(src.ids(), vec![0x51, 0x52, 0x53, 0x54], "the name alone");
    assert_eq!(r.battery.percentage, None, "no battery read");
    assert!(r.device.name_on_keyboard_hex.is_some());
    let src = Src::new(0, ans.clone());
    akm_core::read_policy::set_name_only_schedule();
    let (_, o) = build_report_safe(BCM, None, &src, KbWake::default(), Instant::now());
    assert_eq!(o, SafeRead::Complete);
    assert!(src.ids().is_empty(), "{:x?}", src.ids());
    // A mute keyboard: stop at the first request, never a new try.
    akm_core::read_policy::forget_name_fragments();
    let src = Src::new(0, full());
    akm_core::read_policy::set_name_only_schedule();
    let (_, o) = build_report_safe(BCM, None, &src, KbWake::default(), Instant::now());
    assert_eq!(o, SafeRead::Partial);
    assert!(o.attempted_and_failed());
    assert_eq!(src.ids(), vec![0x51]);
    akm_core::read_policy::set_schedule(Some(false));
    let src = Src::new(0, ans);
    let (_, o) = build_report_safe(BCM, None, &src, KbWake::default(), Instant::now());
    assert_eq!(o, SafeRead::Skipped(Gate::NotDue));
    assert!(!o.attempted_and_failed());
    let got = src.ids();
    assert!(got.is_empty(), "{got:?}");
    akm_core::read_policy::set_schedule(None);
    akm_core::read_policy::note_connection();
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn the_last_hardware_access_of_another_process_spaces_the_first_request() {
    use akm_core::read_policy::{
        hw_stamp_path, monotonic_ms, read_hw_stamp, share_hw_access, stamp_age, write_hw_stamp,
    };
    let _g = serial();
    let d = private_runtime_dir("stamp");
    assert_eq!(stamp_age(1_000, 1_400), Some(Duration::from_millis(400)));
    assert_eq!(stamp_age(1_000, 1_000), Some(Duration::ZERO));
    assert_eq!(stamp_age(1_000, 2_000), None, "old: nothing to wait for");
    assert_eq!(stamp_age(5_000, 1_000), None, "another boot: ignored");
    let stamp = hw_stamp_path();
    assert_eq!(stamp.parent(), lock_path().parent());
    assert_eq!(read_hw_stamp(&stamp), None);
    let breaker = Mutex::new(akm_core::read_policy::Breaker::new());
    let conn = Mutex::new(akm_core::read_policy::ConnState::new());
    let src = Src::new(0, full());

    share_hw_access(true);
    let before = monotonic_ms();
    write_hw_stamp(&stamp, before).unwrap();
    let safe = SafeSource::with_parts(&src, &breaker, &conn);
    let t = Instant::now();
    safe.feature(0x47).unwrap();
    let waited = t.elapsed();
    assert!(
        waited >= MIN_GAP.checked_sub(Duration::from_millis(100)).unwrap(),
        "first request {waited:?} after another process's access: no spacing"
    );
    assert!(read_hw_stamp(&stamp).unwrap() >= before + 900);
    assert!(akm_core::read_policy::last_hw_access().is_some());
    write_hw_stamp(&stamp, monotonic_ms().saturating_sub(5_000)).unwrap();
    let safe = SafeSource::with_parts(&src, &breaker, &conn);
    let t = Instant::now();
    safe.feature(0x47).unwrap();
    assert!(t.elapsed() < Duration::from_millis(500));
    std::fs::write(&stamp, "garbage").unwrap();
    assert_eq!(read_hw_stamp(&stamp), None);

    share_hw_access(false);
    write_hw_stamp(&stamp, monotonic_ms()).unwrap();
    let safe = SafeSource::with_parts(&src, &breaker, &conn);
    let t = Instant::now();
    safe.feature(0x47).unwrap();
    assert!(t.elapsed() < Duration::from_millis(500));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn in_process_contention_honours_wait_and_survives_a_panicking_holder() {
    let _g = serial();
    let d = private_runtime_dir("inproc");
    let (tx, rx) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || {
        let first = try_lock(Duration::ZERO).expect("lock free");
        tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(first);
    });
    rx.recv().unwrap();
    assert!(
        try_lock(Duration::from_millis(1500)).is_some(),
        "wait ignored under local contention"
    );
    h.join().unwrap();
    let held = try_lock(Duration::ZERO).unwrap();
    let t = Instant::now();
    assert!(try_lock(Duration::from_millis(200)).is_none());
    assert!(t.elapsed() >= Duration::from_millis(190) && t.elapsed() < Duration::from_millis(700));
    drop(held);
    let r = std::thread::spawn(|| {
        let _l = try_lock(Duration::ZERO).unwrap();
        panic!("reader failure (expected by the test)");
    })
    .join();
    assert!(r.is_err());
    assert!(
        try_lock(Duration::ZERO).is_some(),
        "poisoned lock = reads refused forever"
    );
    std::fs::remove_dir_all(&d).ok();
}
