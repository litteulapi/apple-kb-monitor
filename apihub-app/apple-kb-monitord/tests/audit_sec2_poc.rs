//! Regressions de l'audit securite 2 (docs/AUDIT-SECURITE-2.md), preuves
//! d'origine inversees, sur un bus de session PRIVE (`dbus-run-session`) :
//!
//! * #202/#203 : le backend lance `<helper> set-fnmode <n>` (jamais `set`),
//!   le programme n'est plus choisi par l'environnement, ni `pkexec` par `$PATH` ;
//! * #203 : N appels `SetFnMode` simultanes = UN SEUL pkexec, les autres sont
//!   refuses (`LimitsExceeded`) ; `SetSwapOptCmd`/`SetIsoLayout` n'existent plus ;
//! * #207 : `SetAlias("--version")` est refuse.
//!
//! Un faux `pkexec` (script du tempdir) journalise ses arguments : rien n'est
//! execute en root.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akm_core::history::{History, SystemClock};
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::alias::AliasBackend;
use apple_kb_monitord::devices::DEVICE_INTERFACE;
use apple_kb_monitord::service::{self, ServeOptions};
use apple_kb_monitord::settings::{HelperBackend, SetError};
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

fn call_i(c: &Connection, m: &str, v: i32) -> zbus::Result<zbus::Message> {
    c.call_method(Some(service::BUS_NAME), DEV, Some(DEVICE_INTERFACE), m, &(v,))
}

fn call_s(c: &Connection, m: &str, v: &str) -> zbus::Result<zbus::Message> {
    c.call_method(Some(service::BUS_NAME), DEV, Some(DEVICE_INTERFACE), m, &(v,))
}

fn inner() {
    let dir = std::env::temp_dir().join(format!("akm-sec2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("pkexec.log");
    let fake = dir.join("pkexec");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$$ $*\" >> {}\nsleep 3\nexit 127\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let helper = dir.join("akm-helper");
    std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
    // L'environnement ne choisit plus rien : ces variables doivent etre ignorees.
    std::env::set_var("APPLE_KB_SETTINGS_HELPER", dir.join("autre"));
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    std::env::set_var("PATH", path);

    let watch = Arc::new(Watch::new());
    let mut k = KbReport::default();
    k.device.mac = Some(MAC.into());
    k.battery.percentage_fine = Some(80.0);
    watch.publish(Snapshot { connected: true, keyboard: Some(k), last_update: 1, ..Default::default() });
    let mut so = ServeOptions::new(watch, Mailbox::new(), Some(Arc::new(History::new(dir.join("h.jsonl"), SystemClock))));
    so.settings = Arc::new(HelperBackend::with_paths(&fake, &helper));
    let alias = Arc::new(RecAlias::default());
    so.alias = alias.clone();
    let _srv = service::serve_with(so).expect("serve");
    std::thread::sleep(Duration::from_millis(300));

    let hs: Vec<_> = (0..N)
        .map(|_| {
            std::thread::spawn(|| {
                let c = Connection::session().unwrap();
                call_i(&c, "SetFnMode", 2).err().map(|e| e.to_string())
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1500));
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(text.lines().count(), 1, "un seul pkexec a la fois : {text}");
    assert!(text.trim_end().ends_with(&format!("{} set-fnmode 2", helper.display())), "{text}");
    let errs: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    let busy = errs.iter().flatten().filter(|e| e.contains("LimitsExceeded")).count();
    assert_eq!(busy, N - 1, "{errs:?}");

    let c = Connection::session().unwrap();
    for m in ["SetSwapOptCmd", "SetIsoLayout"] {
        let e = call_i(&c, m, 1).unwrap_err();
        assert!(e.to_string().contains("UnknownMethod"), "{m}: {e}");
    }
    let e = call_s(&c, "SetAlias", "--version").unwrap_err();
    assert!(e.to_string().contains("InvalidArgs") || e.to_string().contains("Failed"), "{e}");
    assert!(alias.0.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_sec2_regressions_private_bus() {
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
        .args(["--exact", "audit_sec2_regressions_private_bus", "--nocapture", "--test-threads=1"])
        .env(INNER, "1")
        .output()
        .expect("dbus-run-session");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", &text[text.len().saturating_sub(2500)..]);
}
