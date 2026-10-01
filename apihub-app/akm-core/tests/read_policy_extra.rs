//! Tests ciblés de la politique de lecture sûre (#177) : bornes exactes,
//! verrou, espacement, budget, décodage. N'utilise que l'API publique.
//! L'état global (verrou en processus, `XDG_RUNTIME_DIR`) impose de sérialiser.

use akm_core::decode::HidSource;
use akm_core::read_policy::{
    build_report_safe, gate, is_allowed, last_input_age, lock_path, note_input, read_safe,
    try_lock, Gate, SafeRead, SafeSource, ACTIVE_WINDOW, ALLOWED, TRIP_AFTER, BUDGET, LOCK_WAIT, MIN_GAP,
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
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Dossier d'exécution privé pour le verrou : jamais le vrai `hid.lock`.
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
}
impl Src {
    fn new(delay_ms: u64, answers: Vec<(u8, Vec<u8>)>) -> Self {
        Self { calls: RefCell::new(vec![]), delay: Duration::from_millis(delay_ms), answers }
    }
    fn ids(&self) -> Vec<u8> {
        self.calls.borrow().iter().map(|c| c.0).collect()
    }
}
impl HidSource for Src {
    fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
        self.calls.borrow_mut().push((id, Instant::now()));
        std::thread::sleep(self.delay);
        self.answers
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

const BCM: &str = "HID_ID=0005:000005AC:00000256\nHID_NAME=Kb\nHID_UNIQ=04:db:56:ca:42:ee\n";

#[test]
fn constants_are_the_documented_policy() {
    assert_eq!(ALLOWED, [0x47, 0x46, 0x49]);
    assert_eq!(ACTIVE_WINDOW, Duration::from_secs(60));
    // Apple espace ses requêtes de 1 s et coupe après 3 expirations
    // (docs/RE-MACOS-SILICON.md) : jamais en dessous.
    assert_eq!(MIN_GAP, Duration::from_millis(1000));
    assert_eq!(TRIP_AFTER, 3);
    assert_eq!(BUDGET, Duration::from_secs(2));
    assert_eq!(LOCK_WAIT, Duration::from_millis(500));
    // Le registre est la source de vérité : routine = 0x47/0x46/0x49,
    // une fois par connexion = 0x4F/0x51..0x54/0x60 ; rien d'autre.
    assert_eq!(registry::SAFE_READ_IDS, [0x47, 0x46, 0x49]);
    assert_eq!(registry::DAEMON_ONCE_IDS, [0x4F, 0x60]);
    for id in 0..=255u8 {
        let expected = matches!(id, 0x47 | 0x46 | 0x49 | 0x4F | 0x51..=0x54 | 0x60);
        assert_eq!(is_allowed(id), expected, "{id:#x}");
    }
    // Jamais : 0xFE (bascule DFU/écriture), 0x4C, Input 0x01.
    for id in [0xFEu8, 0x4C, 0x01] {
        assert!(!is_allowed(id), "{id:#x} doit rester refusé");
    }
}

#[test]
fn gate_boundaries() {
    assert_eq!(gate(None), Gate::Idle);
    assert_eq!(gate(Some(Duration::ZERO)), Gate::Allowed);
    assert_eq!(gate(Some(ACTIVE_WINDOW - Duration::from_millis(1))), Gate::Allowed);
    assert_eq!(gate(Some(ACTIVE_WINDOW)), Gate::Idle);
    assert_eq!(gate(Some(ACTIVE_WINDOW + Duration::from_millis(1))), Gate::Idle);
    assert_eq!(gate(Some(Duration::from_secs(3600))), Gate::Idle);
}

#[test]
fn input_age_is_measured_from_the_last_note() {
    let _g = serial();
    note_input();
    let a = last_input_age(Instant::now()).unwrap();
    assert!(a < Duration::from_secs(1));
    let b = last_input_age(Instant::now() + Duration::from_secs(30)).unwrap();
    assert!(b >= Duration::from_secs(30) && b < Duration::from_secs(31), "{b:?}");
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
    let first = try_lock(Duration::ZERO).expect("verrou libre");
    assert!(lock_path().is_file(), "le fichier de verrou est créé");
    assert!(try_lock(Duration::ZERO).is_none(), "second lecteur du même processus refusé");
    drop(first);
    let again = try_lock(Duration::ZERO);
    assert!(again.is_some(), "verrou rendu après drop");
    drop(again);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn lock_waits_then_gives_up_on_a_foreign_flock() {
    let _g = serial();
    let d = private_runtime_dir("flock");
    // Autre « processus » : description de fichier distincte, flock exclusif.
    // Le dossier du verrou doit être privé (0700, #208) sinon try_lock refuse.
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(d.join("apple-kb-monitor")).unwrap();
    }
    let other = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path())
        .unwrap();
    // SAFETY: fd valide possédé par `other`.
    assert_eq!(unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
    let t = Instant::now();
    assert!(try_lock(Duration::from_millis(300)).is_none());
    let e = t.elapsed();
    assert!(e >= Duration::from_millis(290), "a abandonné trop tôt : {e:?}");
    assert!(e < Duration::from_millis(900), "a attendu trop longtemps : {e:?}");
    // Sans attente : refus immédiat.
    let t = Instant::now();
    assert!(try_lock(Duration::ZERO).is_none());
    assert!(t.elapsed() < Duration::from_millis(40));
    // Le verrou étranger est libéré en cours d'attente : on l'obtient.
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
    assert!(gap >= MIN_GAP - Duration::from_millis(5), "{gap:?}");
    assert!(gap < MIN_GAP + Duration::from_millis(150), "{gap:?}");
    drop(c);
    // Écart partiel : seul le complément est attendu, pas MIN_GAP en plus.
    let before = src.calls.borrow().len();
    s.feature(0x47).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    s.feature(0x46).unwrap();
    {
        let c = src.calls.borrow();
        let gap = c[before + 1].1.duration_since(c[before].1);
        assert!(gap >= MIN_GAP - Duration::from_millis(5), "{gap:?}");
        assert!(gap < MIN_GAP + Duration::from_millis(60), "{gap:?}");
    }
    // Déjà en retard de plus de MIN_GAP : aucune attente supplémentaire.
    std::thread::sleep(MIN_GAP + Duration::from_millis(100));
    let t = Instant::now();
    s.feature(0x47).unwrap();
    assert!(t.elapsed() < Duration::from_millis(60), "{:?}", t.elapsed());
    assert_eq!(s.sent(), 5);
}

#[test]
fn refused_ids_do_not_count_nor_delay() {
    let src = Src::new(0, vec![]);
    let s = SafeSource::new(&src);
    let t = Instant::now();
    for id in [0u8, 0x45, 0x48, 0x4A, 0xEA, 0xFE] {
        assert_eq!(s.feature(id).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
    assert!(t.elapsed() < Duration::from_millis(40));
    assert_eq!(s.sent(), 0);
    assert!(src.ids().is_empty());
}

fn full() -> Vec<(u8, Vec<u8>)> {
    vec![(0x47, vec![0x47, 80]), (0x46, vec![0x46, 0xA0, 0x0B]), (0x49, vec![0x49, 0x89, 0x0B])]
}

#[test]
fn read_safe_stores_raw_hex_and_decodes() {
    let src = Src::new(0, full());
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert_eq!(src.ids(), vec![0x47, 0x46, 0x49]);
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
        let src = Src::new(0, vec![(0x47, vec![0x47, p]), (0x46, vec![0x46, 0, 0]), (0x49, vec![0x49, 0, 0])]);
        let mut r = KbReport::default();
        read_safe(&src, &mut r);
        assert_eq!(r.battery.percentage, ok.then_some(f64::from(p)), "pct {p}");
    }
    // Un pourcentage déjà connu (noyau) n'est pas écrasé.
    let src = Src::new(0, full());
    let mut r = KbReport::default();
    r.battery.percentage = Some(42.0);
    read_safe(&src, &mut r);
    assert_eq!(r.battery.percentage, Some(42.0));
}

#[test]
fn read_safe_voltage_bounds() {
    for (mv, ok) in [(1499u16, false), (1500, true), (2976, true), (3700, true), (3701, false), (0, false), (65535, false)] {
        let le = mv.to_le_bytes();
        let src = Src::new(0, vec![(0x47, vec![0x47, 50]), (0x46, vec![0x46, le[0], le[1]]), (0x49, vec![0x49, 0, 0])]);
        let mut r = KbReport::default();
        read_safe(&src, &mut r);
        assert_eq!(r.battery.voltage, ok.then(|| f64::from(mv) / 1000.0), "{mv} mV");
    }
}

#[test]
fn read_safe_short_answers() {
    // 0x46 avec un seul octet utile : pas de tension, mais la lecture continue.
    let src = Src::new(0, vec![(0x47, vec![0x47, 70]), (0x46, vec![0x46, 0xA0]), (0x49, vec![0x49, 1, 2])]);
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert_eq!(r.battery.voltage, None);
    assert_eq!(r.raw.get("0x46").map(String::as_str), Some("a0"));
    assert_eq!(src.ids(), vec![0x47, 0x46, 0x49]);
    // Réponse d'un seul octet (l'id seul) : ignorée sans clé brute, lecture poursuivie.
    let src = Src::new(0, vec![(0x47, vec![0x47]), (0x46, vec![0x46, 0xA0, 0x0B]), (0x49, vec![0x49])]);
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
    assert!(!r.raw.contains_key("0x47") && !r.raw.contains_key("0x49"));
    assert_eq!(r.battery.percentage, None);
    assert_eq!(r.battery.voltage, Some(2.976));
    assert_eq!(src.ids(), vec![0x47, 0x46, 0x49]);
}

#[test]
fn read_safe_stops_at_first_failure_and_flags_incomplete() {
    for missing in [0x47u8, 0x46, 0x49] {
        let ans: Vec<_> = full().into_iter().filter(|(i, _)| *i != missing).collect();
        let src = Src::new(0, ans);
        let mut r = KbReport::default();
        assert_eq!(read_safe(&src, &mut r), SafeRead::Partial, "manque {missing:#x}");
        assert!(r.incomplete);
        let n = [0x47u8, 0x46, 0x49].iter().position(|i| *i == missing).unwrap();
        assert_eq!(src.ids().len(), n + 1, "arrêt au premier échec");
    }
}

#[test]
fn read_safe_stops_when_the_budget_is_spent() {
    // 1,1 s par requête : la 1re part, la 2e (t ~ 1,35 s) aussi, la 3e (t ~ 2,7 s) non.
    let src = Src::new(1100, full());
    let mut r = KbReport::default();
    let t = Instant::now();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Partial);
    assert_eq!(src.ids(), vec![0x47, 0x46]);
    assert!(r.incomplete);
    assert!(t.elapsed() < Duration::from_millis(3500));
}

#[test]
fn build_report_safe_paths() {
    let _g = serial();
    let d = private_runtime_dir("build");
    let wake = KbWake { last_age_s: Some(1.5), count: 7 };

    // Famille inconnue : jamais d'E/S, politique « Allowed » sans lecture.
    let src = Src::new(0, full());
    let (r, o) = build_report_safe("", None, &src, wake.clone(), Instant::now());
    assert_eq!(o, SafeRead::Skipped(Gate::Allowed));
    assert!(src.ids().is_empty());
    assert!(r.bluetooth.connected);
    assert_eq!(r.wake, wake);

    // BCM2042 actif : lecture complète (routine + 0x4F/0x60 une seule fois
    // par connexion), % fin = %.
    note_input();
    let mut ans = full();
    ans.push((0x4F, vec![0x4F, 1, 2, 3, 4]));
    ans.push((0x60, vec![0x60, 5, 6]));
    // puis le nom stocké dans le clavier 0x51-0x54, priorité basse (#248) :
    // ce qui ne tient pas dans le budget attend la rafale suivante.
    for id in [0x51u8, 0x52, 0x53, 0x54] {
        ans.push((id, vec![id, b'a', 0, 0, 0, 0, 0, 0, 0]));
    }
    let src = Src::new(0, ans);
    let (r, o) = build_report_safe(BCM, None, &src, wake.clone(), Instant::now());
    assert_eq!(o, SafeRead::Complete);
    let ids = src.ids();
    assert_eq!(ids[..5], [0x47, 0x46, 0x49, 0x4F, 0x60]);
    assert!(
        [0x51u8, 0x52, 0x53, 0x54].starts_with(&ids[5..]),
        "{ids:x?}"
    );
    assert_eq!(r.battery.percentage, Some(80.0));
    assert_eq!(r.battery.percentage_fine, Some(80.0));
    assert_eq!(r.wake, wake);
    assert!(r.bluetooth.connected);

    // BCM2042 inactif : rien n'est demandé.
    let src = Src::new(0, full());
    let (_, o) = build_report_safe(BCM, None, &src, wake.clone(), Instant::now() + ACTIVE_WINDOW * 2);
    assert_eq!(o, SafeRead::Skipped(Gate::Idle));
    assert!(src.ids().is_empty());

    // BCM2042 actif mais verrou occupé : Busy, rien n'est demandé.
    let held = try_lock(Duration::ZERO).unwrap();
    let src = Src::new(0, full());
    let (r, o) = build_report_safe(BCM, None, &src, wake, Instant::now());
    assert_eq!(o, SafeRead::Skipped(Gate::Busy));
    assert!(src.ids().is_empty());
    assert_eq!(r.battery.percentage_fine, r.battery.percentage);
    drop(held);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn in_process_contention_honours_wait_and_survives_a_panicking_holder() {
    let _g = serial();
    let d = private_runtime_dir("inproc");
    // Un lecteur du même processus libère pendant l'attente : on l'obtient.
    let (tx, rx) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || {
        let first = try_lock(Duration::ZERO).expect("verrou libre");
        tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(first);
    });
    rx.recv().unwrap();
    assert!(try_lock(Duration::from_millis(1500)).is_some(), "wait ignoré en contention locale");
    h.join().unwrap();
    // Sans libération, l'attente est bornée.
    let held = try_lock(Duration::ZERO).unwrap();
    let t = Instant::now();
    assert!(try_lock(Duration::from_millis(200)).is_none());
    assert!(t.elapsed() >= Duration::from_millis(190) && t.elapsed() < Duration::from_millis(700));
    drop(held);
    // Un lecteur qui panique ne doit pas interdire toute lecture ultérieure.
    let r = std::thread::spawn(|| {
        let _l = try_lock(Duration::ZERO).unwrap();
        panic!("lecteur en panne (attendu par le test)");
    })
    .join();
    assert!(r.is_err());
    assert!(try_lock(Duration::ZERO).is_some(), "verrou empoisonné = lectures refusées à jamais");
    std::fs::remove_dir_all(&d).ok();
}
