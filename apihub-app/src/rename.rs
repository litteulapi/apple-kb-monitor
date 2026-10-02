//! "Rename keyboard" field of the window (#141): sets the BlueZ alias through
//! the daemon (`SetAlias`), or directly when the daemon is absent. Nothing is
//! written into the keyboard.

use std::sync::{Arc, Mutex};

use akm_core::alias;
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::alias::{rename, BluezAlias};
use apple_kb_monitord::client;

/// Outcome shown under the field: `(ok, message)`.
pub type Status = Arc<Mutex<Option<(bool, String)>>>;

/// Local check before any D-Bus call: `Ok("")` = restore the own name.
pub fn check(input: &str) -> Result<String, String> {
    alias::validate(input).map_err(|e| e.to_string())
}

/// Apply `name` (already `check`ed; `""` resets) to keyboard `mac` on a
/// worker thread, report into `status`.
pub fn submit(mac: String, name: String, status: Status, ctx: eframe::egui::Context) {
    let _ = std::thread::Builder::new()
        .name("rename".into())
        .spawn(move || {
            let res = apply(&mac, &name);
            *status.lock().unwrap_or_else(|e| e.into_inner()) = Some(match res {
                Ok(now) => (true, crate::i18n::trf("Name: {}", &[&now])),
                Err(e) => (false, e),
            });
            ctx.request_repaint();
        });
}

fn apply(mac: &str, name: &str) -> Result<String, String> {
    if let Ok(conn) = zbus::blocking::Connection::session() {
        if client::daemon_present(&conn) {
            return client::set_alias(&conn, mac, name).map_err(|e| e.to_string());
        }
    }
    // Daemon absent (local fallback): BlueZ directly.
    rename(
        &BluezAlias::default(),
        &Mailbox::default(),
        mac,
        name,
        "apihub-app window (daemon absent)",
    )
    .map(Option::unwrap_or_default)
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_trims_validates_and_allows_reset() {
        assert_eq!(check("  Bureau ").unwrap(), "Bureau");
        assert_eq!(check("   ").unwrap(), "");
        assert!(check("a\nb").is_err());
        assert!(check(&"x".repeat(65)).is_err());
    }
}
