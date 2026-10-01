//! Audit securite 2 (docs/AUDIT-SECURITE-2.md) : preuves de concept
//! INOFFENSIVES sur un bus de session PRIVE (`dbus-run-session`).
//!
//! * PoC-A : le vrai `HelperBackend` du demon lance `pkexec` resolu par `$PATH`,
//!   avec un programme choisi par `APPLE_KB_SETTINGS_HELPER`, et des arguments
//!   (`set fnmode 2`) que `akm-helper` refuse. Un faux `pkexec` (script du
//!   tempdir) journalise ses arguments : rien n'est execute en root.
//! * PoC-B : N appels D-Bus `SetFnMode` simultanes d'un client quelconque de la
//!   session = N `pkexec` (donc N demandes d'authentification admin) en
//!   parallele, sans limite ni file unique.
//! * PoC-C : `SetAlias` accepte un nom qui commence par `--` et un nom HTML.
//!
//! Lancer : `cargo test -p apple-kb-monitord --test audit_sec2_poc -- --nocapture`

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akm_core::history::{History, SystemClock};
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::alias::AliasBackend;
use apple_kb_monitord::devices::DEVICE_INTERFACE;
use apple_kb_monitord::service::{self, ServeOptions};
use apple_kb_monitord::settings::{HelperBackend, SetError, HELPER_ENV};
use zbus::blocking::Connection;

const INNER: &str = "AKM_AUDIT_SEC2_INNER";
const MAC: &str = "04:DB:56:CA:42:EE";
const DEV: &str = "/com/agenceapi/AppleKbMonitor1/devices/04_DB_56_CA_42_EE";
const N: usize = 12;

#[derive(Debug, Default)]
struct RecAlias(Mutex<Vec<String>>);
impl AliasBackend for RecAlias {
    fn get(&self, _: &str) -> Option<String> {
        self.0.lock().unwrap().last().cloned()
    }
    fn set(&self, _: &str, a: &str) -> Result<(), SetError> {
        self.0.lock().unwrap().push(a.into());
        Ok(())
    }
}

fn inner() {
    let dir = std::env::temp_dir().join(format!("akm-sec2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let log = dir.join("pkexec.log");
    // Faux pkexec : journalise argv, attend 3 s (= dialogue polkit ouvert), echoue.
    let fake = dir.join("bin/pkexec");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$$ $*\" >> {}\nsleep 3\nexit 127\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Programme arbitraire choisi par l'environnement du demon.
    let chosen = dir.join("not-the-helper");
    std::fs::write(&chosen, "#!/bin/sh\nexit 0\n").unwrap();
    std::env::set_var(HELPER_ENV, &chosen);
    let path = format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
    std::env::set_var("PATH", path);

    let watch = Arc::new(Watch::new());
    let mut k = KbReport::default();
    k.device.mac = Some(MAC.into());
    k.battery.percentage_fine = Some(80.0);
    watch.publish(Snapshot { connected: true, keyboard: Some(k), last_update: 1, ..Default::default() });
    let mut so = ServeOptions::new(watch, Mailbox::new(), Some(Arc::new(History::new(dir.join("h.jsonl"), SystemClock))));
    so.settings = Arc::new(HelperBackend); // le VRAI backend
    let alias = Arc::new(RecAlias::default());
    so.alias = alias.clone();
    let _srv = service::serve_with(so).expect("serve");
    std::thread::sleep(Duration::from_millis(300));

    // PoC-B : N appels simultanes depuis un client quelconque de la session.
    let t0 = Instant::now();
    let hs: Vec<_> = (0..N)
        .map(|_| {
            std::thread::spawn(|| {
                let c = Connection::session().unwrap();
                c.call_method(Some(service::BUS_NAME), DEV, Some(DEVICE_INTERFACE), "SetFnMode", &(2i32,))
                    .err()
                    .map(|e| e.to_string())
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1500));
    let concurrent = std::fs::read_to_string(&log).unwrap_or_default().lines().count();
    let errs: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    println!("POC-A argv vus par pkexec (PATH) :\n{}", text.lines().take(2).collect::<Vec<_>>().join("\n"));
    println!("POC-B pkexec simultanes apres 1,5 s : {concurrent}/{N} (duree totale {:?})", t0.elapsed());
    println!("POC-B erreur rendue au client : {:?}", errs[0]);
    assert!(text.contains(&format!("{} set fnmode 2", chosen.display())), "{text}");
    assert_eq!(concurrent, N, "pkexec non serialise");

    // PoC-C : noms acceptes.
    let c = Connection::session().unwrap();
    for name in ["--version", "<img src=\"http://127.0.0.1:9/x\">"] {
        let r = c.call_method(Some(service::BUS_NAME), DEV, Some(DEVICE_INTERFACE), "SetAlias", &(name,));
        println!("POC-C SetAlias({name:?}) -> {}", if r.is_ok() { "ACCEPTE" } else { "refuse" });
        assert!(r.is_ok());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_sec2_poc_private_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    if Command::new("dbus-run-session").arg("--version").output().is_err() {
        eprintln!("SKIP: dbus-run-session absent");
        return;
    }
    let out = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "audit_sec2_poc_private_bus", "--nocapture", "--test-threads=1"])
        .env(INNER, "1")
        .output()
        .expect("dbus-run-session");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    println!("{}", text.lines().filter(|l| l.starts_with("POC")).collect::<Vec<_>>().join("\n"));
    assert!(out.status.success(), "{}", &text[text.len().saturating_sub(2500)..]);
}
