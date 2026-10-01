//! AUDIT FINAL (2026-10-01, #256) — tests de non-régression sur le fichier
//! d'état du disjoncteur lu par `akm-hid-control` (root) depuis
//! `/run/user/<uid>/…`. À l'origine ils prouvaient le défaut ; ils prouvent
//! maintenant la correction.
//!
//! Inoffensif : tout se passe dans un répertoire temporaire qui joue
//! `/run/user` et `/proc`, aucun socket, aucun processus réel.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use akm_helper::breaker_state::{self, Allow, BreakerState, Refuse, Verdict, Writer};
use akm_helper::hidctl::{self, Env, Mac};

const KB: &str = "04:DB:56:CA:42:EE";

fn uid() -> u32 {
    // SAFETY: getuid(2) has no failure mode.
    unsafe { libc::getuid() }
}

fn fake_run_user(tag: &str, content: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("akm-audit-breaker-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join(uid().to_string()).join(breaker_state::DIR_NAME);
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join(breaker_state::FILE_NAME);
    fs::write(&p, content).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    root
}

fn env<'a>(
    root: &'a Path,
    alive: &'a dyn Fn(u32, Option<Writer>) -> bool,
    active: &'a dyn Fn() -> Option<u32>,
) -> Env<'a> {
    Env {
        proc_root: root.join("proc"),
        hid_root: root.join("hid"),
        allowed_exes: &[],
        required_uid: 0,
        daemon_pid: &|| Err("unused".into()),
        run_user_root: root.to_path_buf(),
        daemon_alive: alive,
        active_uid: active,
    }
}

/// D1 corrigé — un `written_unix` dans le futur (au-delà de 5 s de dérive)
/// n'est plus « frais » : sans démon vivant, il ne bloque plus rien.
#[test]
fn future_dated_open_state_no_longer_blocks_without_a_daemon() {
    let st = BreakerState::parse(&format!(
        "schema=2\nmac={KB}\nopen=1\ncounter=3\nwritten_unix=999999999999\npid=1\nstarttime=9\n"
    ))
    .unwrap();
    let now = breaker_state::now_unix();
    assert!(st.ahead_of(now).is_some());
    let never_alive = |_: Option<Writer>| false;
    let v = breaker_state::verdict(&Ok(Some(st.clone())), KB, now, &never_alive);
    assert_eq!(v, Verdict::Allow(Allow::NoDaemon));
    // fail closed tant que son auteur tourne
    let alive = |_: Option<Writer>| true;
    let v = breaker_state::verdict(&Ok(Some(st)), KB, now, &alive);
    assert!(matches!(v, Verdict::Refuse(Refuse::Future { .. })), "{v:?}");
}

/// D1 bis corrigé — à travers le vrai chemin de lecture de `akm-hid-control`.
#[test]
fn hid_control_reader_ignores_a_forged_future_dated_file_from_a_dead_daemon() {
    let root = fake_run_user(
        "future",
        &format!("schema=2\nmac={KB}\nopen=1\ncounter=3\nwritten_unix=999999999999\npid=4242\nstarttime=9\n"),
    );
    let dead = |_uid: u32, _w: Option<Writer>| false;
    let me = || Some(uid());
    let e = env(&root, &dead, &me);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), breaker_state::now_unix());
    assert_eq!(v, Verdict::Allow(Allow::NoDaemon));
    fs::remove_dir_all(&root).unwrap();
}

/// D1 ter — contraste inchangé : daté d'il y a une heure, démon mort = ignoré.
#[test]
fn past_dated_open_state_from_a_dead_daemon_is_ignored() {
    let old = breaker_state::now_unix() - 3600;
    let root = fake_run_user(
        "past",
        &format!(
            "schema=2\nmac={KB}\nopen=1\ncounter=3\nwritten_unix={old}\npid=4242\nstarttime=9\n"
        ),
    );
    let dead = |_uid: u32, _w: Option<Writer>| false;
    let me = || Some(uid());
    let e = env(&root, &dead, &me);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), breaker_state::now_unix());
    assert!(v.allows(), "{v:?}");
    fs::remove_dir_all(&root).unwrap();
}

/// D2 corrigé — le lecteur root ne lit QUE l'utilisateur actif du siège :
/// l'état `open=1` frais d'un autre compte (démon vivant) ne bloque pas le
/// clavier de l'utilisateur actif ; sans utilisateur actif, rien ne part.
#[test]
fn another_accounts_open_breaker_does_not_block_the_active_user() {
    let now = breaker_state::now_unix();
    let root = fake_run_user(
        "other",
        &format!(
            "schema=2\nmac={KB}\nopen=1\ncounter=3\nwritten_unix={now}\npid=4242\nstarttime=9\n"
        ),
    );
    let alive = |_uid: u32, _w: Option<Writer>| true;
    // l'utilisateur actif est un autre compte (sans état publié)
    let other = || Some(uid() + 1);
    let e = env(&root, &alive, &other);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), now);
    assert_eq!(v, Verdict::Allow(Allow::NoState), "{v:?}");
    // l'utilisateur actif est bien le propriétaire : son disjoncteur s'applique
    let me = || Some(uid());
    let e = env(&root, &alive, &me);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), now);
    assert_eq!(v, Verdict::Refuse(Refuse::Open { counter: 3 }));
    // personne d'actif : fail closed
    let nobody = || None;
    let e = env(&root, &alive, &nobody);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), now);
    assert_eq!(v, Verdict::Refuse(Refuse::NoActiveUser));
    fs::remove_dir_all(&root).unwrap();
}

/// D3 corrigé — un processus qui s'est renommé `apple-kb-monito`
/// (`prctl(PR_SET_NAME)`) n'est plus pris pour le démon : il faut l'exe du
/// démon, l'uid, et l'heure de démarrage de l'auteur de l'état.
#[test]
fn daemon_liveness_needs_exe_uid_and_start_time() {
    let root = std::env::temp_dir().join(format!("akm-audit-proc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let mk = |pid: u32, exe: &str, start: u64| {
        let p = root.join(pid.to_string());
        fs::create_dir_all(&p).unwrap();
        std::os::unix::fs::symlink(exe, p.join("exe")).unwrap();
        fs::write(p.join("comm"), format!("{}\n", breaker_state::DAEMON_COMM)).unwrap();
        fs::write(
            p.join("status"),
            format!("Name:\tx\nUid:\t{u}\t{u}\t{u}\t{u}\n", u = uid()),
        )
        .unwrap();
        let mid = ["0"; 18].join(" ");
        fs::write(
            p.join("stat"),
            format!("{pid} (apple-kb-monito) S {mid} {start} 0\n"),
        )
        .unwrap();
    };
    mk(777, "/home/x/fake", 5);
    assert!(!breaker_state::daemon_alive_in(
        &root,
        Some((777, None)),
        uid()
    ));
    assert!(!breaker_state::daemon_alive_in(&root, None, uid()));
    mk(778, breaker_state::DAEMON_EXE, 6);
    assert!(breaker_state::daemon_alive_in(
        &root,
        Some((778, Some(6))),
        uid()
    ));
    assert!(
        !breaker_state::daemon_alive_in(&root, Some((778, Some(7))), uid()),
        "pid recyclé"
    );
    assert!(
        !breaker_state::daemon_alive_in(&root, Some((778, Some(6))), uid() + 1),
        "autre uid"
    );
    fs::remove_dir_all(&root).unwrap();
}
