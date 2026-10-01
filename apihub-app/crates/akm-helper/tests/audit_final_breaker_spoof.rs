//! AUDIT FINAL (2026-10-01) — preuves de concept sur le fichier d'état du
//! disjoncteur lu par `akm-hid-control` (root) depuis `/run/user/<uid>/…`.
//!
//! Inoffensif : tout se passe dans un répertoire temporaire qui joue
//! `/run/user`, aucun socket, aucun processus réel.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use akm_helper::breaker_state::{self, BreakerState, Refuse, Verdict};
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

fn env<'a>(root: &'a PathBuf, alive: &'a dyn Fn(u32, Option<u32>) -> bool) -> Env<'a> {
    Env {
        proc_root: root.join("proc"),
        hid_root: root.join("hid"),
        allowed_exes: &[],
        required_uid: 0,
        daemon_pid: &|| Err("unused".into()),
        run_user_root: root.clone(),
        daemon_alive: alive,
    }
}

/// D1 — Un `written_unix` dans le futur n'est JAMAIS périmé (`age` sature à 0) :
/// un fichier `open=1` forgé (ou laissé par un démon mort dont l'horloge
/// avançait) bloque SUSPEND/EXIT_SUSPEND pour toujours, démon vivant ou non.
#[test]
fn future_dated_open_state_is_never_stale_and_blocks_without_any_daemon() {
    let st = BreakerState::parse(&format!(
        "schema=1\nmac={KB}\nopen=1\ncounter=3\nwritten_unix=999999999999\npid=1\n"
    ))
    .unwrap();
    // 12 chiffres passent le parseur ; l'état est "frais" à n'importe quelle date.
    assert!(!st.is_stale(breaker_state::now_unix()));
    assert!(!st.is_stale(u64::from(u32::MAX)));
    let never_alive = |_: Option<u32>| false;
    let v = breaker_state::verdict(&Ok(Some(st)), KB, breaker_state::now_unix(), &never_alive);
    assert_eq!(v, Verdict::Refuse(Refuse::Open { counter: 3 }), "refusé alors qu'aucun démon ne tourne");
}

/// D1 bis — même chose à travers le vrai chemin de lecture de `akm-hid-control`
/// (`published_breakers` + `breaker_verdict`) sur un faux `/run/user`.
#[test]
fn hid_control_reader_trusts_a_forged_future_dated_file_from_a_dead_daemon() {
    let root = fake_run_user(
        "future",
        &format!("schema=1\nmac={KB}\nopen=1\ncounter=3\nwritten_unix=999999999999\npid=4242\n"),
    );
    let dead = |_uid: u32, _pid: Option<u32>| false;
    let e = env(&root, &dead);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), breaker_state::now_unix());
    assert!(matches!(v, Verdict::Refuse(Refuse::Open { .. })), "{v:?}");
    fs::remove_dir_all(&root).unwrap();
}

/// D1 ter — contraste : le même fichier daté d'il y a une heure est bien
/// ignoré quand le démon est mort (comportement documenté).
#[test]
fn past_dated_open_state_from_a_dead_daemon_is_ignored() {
    let old = breaker_state::now_unix() - 3600;
    let root = fake_run_user(
        "past",
        &format!("schema=1\nmac={KB}\nopen=1\ncounter=3\nwritten_unix={old}\npid=4242\n"),
    );
    let dead = |_uid: u32, _pid: Option<u32>| false;
    let e = env(&root, &dead);
    let v = hidctl::breaker_verdict(&e, Mac::parse(KB).unwrap(), breaker_state::now_unix());
    assert!(v.allows(), "{v:?}");
    fs::remove_dir_all(&root).unwrap();
}

/// D2 — Le lecteur root agrège TOUS les `/run/user/<uid>` (`combine` : un
/// refus suffit). Le fichier d'un autre compte local, dont il est propriétaire
/// (contrôle `read_user_file` satisfait), suffit à bloquer l'émission vers le
/// clavier d'un autre utilisateur. Ici un seul uid est disponible sans root :
/// on montre que `combine` retient le refus parmi des autorisations.
#[test]
fn one_refusal_among_many_allows_wins() {
    use breaker_state::Allow;
    let v = breaker_state::combine([
        Verdict::Allow(Allow::Closed),
        Verdict::Allow(Allow::NoState),
        Verdict::Refuse(Refuse::Open { counter: 3 }),
        Verdict::Allow(Allow::OtherKeyboard),
    ]);
    assert!(matches!(v, Verdict::Refuse(_)));
}

/// D3 — `Unreadable` + démon "vivant" : le test de vie par `comm` est
/// usurpable (`prctl(PR_SET_NAME)` est libre pour tout processus) ; ici on
/// montre seulement que `daemon_alive_in` ne regarde que `comm` + uid.
#[test]
fn daemon_liveness_is_only_comm_and_uid() {
    let root = std::env::temp_dir().join(format!("akm-audit-proc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let p = root.join("777");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("comm"), format!("{}\n", breaker_state::DAEMON_COMM)).unwrap();
    fs::write(p.join("status"), format!("Name:\tx\nUid:\t{u}\t{u}\t{u}\t{u}\n", u = uid())).unwrap();
    assert!(breaker_state::daemon_alive_in(&root, Some(777), uid()));
    assert!(breaker_state::daemon_alive_in(&root, None, uid()));
    fs::remove_dir_all(&root).unwrap();
}
