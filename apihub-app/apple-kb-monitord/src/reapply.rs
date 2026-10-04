//! Settings remembered per keyboard and put back at its reconnection.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use akm_core::device_settings::{plan, DeviceSettings, Plan, Reapply};
use akm_core::hid_params::{Param, UNKNOWN};

use crate::notify::Notification;
use crate::settings::SettingsBackend;

type Notify = Arc<dyn Fn(Notification) + Send + Sync>;

/// A mode waiting for the "Apply" button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub mac: String,
    pub fn_mode: i32,
}

pub struct Reapplier {
    memory: Mutex<DeviceSettings>,
    path: Option<PathBuf>,
    backend: Arc<dyn SettingsBackend>,
    policy: Reapply,
    notify: Notify,
    pending: Mutex<Option<Pending>>,
}

impl std::fmt::Debug for Reapplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reapplier")
            .field("policy", &self.policy)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Reapplier {
    pub fn new(
        path: Option<PathBuf>,
        backend: Arc<dyn SettingsBackend>,
        policy: Reapply,
    ) -> Arc<Self> {
        Self::with_notifier(path, backend, policy, Arc::new(crate::notify::deliver))
    }

    /// [`Self::new`] with the notification sink given (tests).
    pub fn with_notifier(
        path: Option<PathBuf>,
        backend: Arc<dyn SettingsBackend>,
        policy: Reapply,
        notify: Notify,
    ) -> Arc<Self> {
        let memory = path
            .as_deref()
            .map(DeviceSettings::load)
            .unwrap_or_default();
        Arc::new(Self {
            memory: Mutex::new(memory),
            path,
            backend,
            policy,
            notify,
            pending: Mutex::new(None),
        })
    }

    /// The user set Fn mode `mode` through the object of keyboard `mac`.
    pub fn remember(&self, mac: &str, mode: i32) {
        let mut m = lock(&self.memory);
        if m.remember_fn_mode(mac, mode) {
            tracing::info!("Fn mode {mode} remembered for {mac}");
            if let Some(p) = self.path.as_deref() {
                if let Err(e) = m.save(p) {
                    tracing::warn!("cannot save {}: {e}", p.display());
                }
            }
        }
    }

    pub fn remembered(&self, mac: &str) -> Option<i32> {
        lock(&self.memory).fn_mode(mac)
    }

    fn live(&self) -> Option<i32> {
        Some(self.backend.get(Param::FnMode)).filter(|v| *v != UNKNOWN)
    }

    pub fn on_connected(self: &Arc<Self>, mac: &str, name: &str) -> Plan {
        let live = self.live();
        let p = plan(self.remembered(mac), live, self.policy);
        match p {
            Plan::Nothing => {}
            Plan::Ask(want) => {
                tracing::info!(
                    "{mac} reconnected: Fn mode {want} remembered, {live:?} in effect; asking"
                );
                *lock(&self.pending) = Some(Pending {
                    mac: mac.to_ascii_uppercase(),
                    fn_mode: want,
                });
                (self.notify)(crate::notify::reapply_notification(
                    name,
                    want,
                    live.unwrap_or(UNKNOWN),
                ));
            }
            Plan::Apply(want) => {
                tracing::info!(
                    "{mac} reconnected: Fn mode {want} remembered, {live:?} in effect; applying"
                );
                self.apply_in_background(want);
            }
        }
        p
    }

    fn apply_in_background(self: &Arc<Self>, mode: i32) {
        let me = self.clone();
        let _ = std::thread::Builder::new()
            .name("kb-reapply".into())
            .spawn(move || {
                me.apply(mode);
            });
    }

    /// Put `mode` in effect through the settings backend (polkit helper).
    pub fn apply(&self, mode: i32) -> bool {
        match crate::settings::set(self.backend.as_ref(), Param::FnMode, mode) {
            Ok(()) => {
                tracing::info!("remembered Fn mode {mode} applied");
                crate::osd::poke();
                true
            }
            Err(e) => {
                tracing::warn!("remembered Fn mode {mode} not applied: {e}");
                false
            }
        }
    }

    /// The "Apply" button was pressed: apply what was offered, once.
    pub fn apply_pending(&self) -> Option<Pending> {
        let p = lock(&self.pending).take()?;
        self.apply(p.fn_mode);
        Some(p)
    }
}

static GLOBAL: OnceLock<Arc<Reapplier>> = OnceLock::new();

/// Make `r` the one the notification button reaches (done once by `main`).
pub fn install(r: Arc<Reapplier>) {
    let _ = GLOBAL.set(r);
}

/// The "Apply" button of the notification.
pub fn apply_pending() {
    if let Some(r) = GLOBAL.get() {
        if r.apply_pending().is_none() {
            tracing::info!("reapply: nothing pending");
        }
    } else {
        tracing::warn!("reapply: not started");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SetError;

    const A: &str = "AA:BB:CC:DD:EE:F1";
    const B: &str = "AA:BB:CC:DD:EE:F2";

    struct FakeHidApple {
        fn_mode: Mutex<i32>,
        applied: Mutex<Vec<i32>>,
    }

    impl SettingsBackend for FakeHidApple {
        fn get(&self, p: Param) -> i32 {
            match p {
                Param::FnMode => *self.fn_mode.lock().unwrap(),
                _ => UNKNOWN,
            }
        }
        fn apply(&self, p: Param, v: i32) -> Result<(), SetError> {
            assert_eq!(p, Param::FnMode);
            *self.fn_mode.lock().unwrap() = v;
            self.applied.lock().unwrap().push(v);
            Ok(())
        }
    }

    type Notes = Arc<Mutex<Vec<Notification>>>;

    fn setup(
        policy: Reapply,
        live: i32,
        path: Option<PathBuf>,
    ) -> (Arc<Reapplier>, Arc<FakeHidApple>, Notes) {
        let hid = Arc::new(FakeHidApple {
            fn_mode: Mutex::new(live),
            applied: Mutex::new(Vec::new()),
        });
        let notes: Notes = Arc::new(Mutex::new(Vec::new()));
        let n = notes.clone();
        let r = Reapplier::with_notifier(
            path,
            hid.clone(),
            policy,
            Arc::new(move |x| n.lock().unwrap().push(x)),
        );
        (r, hid, notes)
    }

    #[test]
    fn two_keyboards_each_get_their_own_mode_back_at_reconnection() {
        let (r, hid, notes) = setup(Reapply::Ask, 1, None);
        r.remember(A, 2);
        *hid.fn_mode.lock().unwrap() = 2;
        r.remember(B, 1);
        *hid.fn_mode.lock().unwrap() = 1;
        assert_eq!(r.on_connected(B, "Salon"), Plan::Nothing);
        assert!(notes.lock().unwrap().is_empty());
        assert_eq!(r.on_connected(A, "Desk"), Plan::Ask(2));
        assert!(
            hid.applied.lock().unwrap().is_empty(),
            "no write without the user"
        );
        {
            let n = notes.lock().unwrap();
            assert_eq!(n.len(), 1);
            assert_eq!(n[0].event, crate::notify::Event::SettingsReapply);
            assert!(n[0].action_list().contains(&"apply".to_string()));
        }
        assert_eq!(
            r.apply_pending(),
            Some(Pending {
                mac: A.into(),
                fn_mode: 2
            })
        );
        assert_eq!(*hid.applied.lock().unwrap(), [2]);
        assert_eq!(r.apply_pending(), None, "once");
        assert_eq!(r.on_connected(B, "Salon"), Plan::Ask(1));
        r.apply_pending();
        assert_eq!(*hid.applied.lock().unwrap(), [2, 1]);
        assert_eq!(notes.lock().unwrap().len(), 2);
    }

    #[test]
    fn policies_auto_and_off() {
        let (r, hid, notes) = setup(Reapply::Auto, 1, None);
        r.remember(A, 2);
        assert_eq!(r.on_connected(A, "Desk"), Plan::Apply(2));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while hid.applied.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < end, "never applied");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(*hid.applied.lock().unwrap(), [2]);
        assert!(notes.lock().unwrap().is_empty(), "auto: no question");
        assert_eq!(
            r.on_connected(A, "Desk"),
            Plan::Nothing,
            "already in effect"
        );

        let (r, hid, notes) = setup(Reapply::Off, 1, None);
        r.remember(A, 2);
        assert_eq!(r.on_connected(A, "Desk"), Plan::Nothing);
        assert!(hid.applied.lock().unwrap().is_empty() && notes.lock().unwrap().is_empty());
        let (r, hid, _) = setup(Reapply::Auto, UNKNOWN, None);
        r.remember(A, 2);
        assert_eq!(r.on_connected(A, "Desk"), Plan::Nothing);
        assert!(hid.applied.lock().unwrap().is_empty());
        assert_eq!(r.on_connected(B, "Salon"), Plan::Nothing);
    }

    #[test]
    fn the_memory_is_kept_across_restarts() {
        let dir = std::env::temp_dir().join(format!("akm-reapply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("device-settings.json");
        let (r, _, _) = setup(Reapply::Ask, 1, Some(path.clone()));
        r.remember(A, 2);
        r.remember(B, 1);
        drop(r);
        let (again, _, _) = setup(Reapply::Ask, 1, Some(path));
        assert_eq!(
            (again.remembered(A), again.remembered(B)),
            (Some(2), Some(1))
        );
        assert_eq!(again.on_connected(A, "Desk"), Plan::Ask(2));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
