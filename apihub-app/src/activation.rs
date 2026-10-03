//! Bring the open window to the front when a second launch, the tray icon or
//! D-Bus `Activate` asks for it (#270).
//!
//! Under Wayland a client cannot raise itself: `xdg_toplevel` has no
//! "unminimize" and winit's `focus_window` does nothing. The compositor only
//! activates a surface through `xdg_activation_v1.activate(token, surface)`,
//! with a token issued for a user action (the launcher, the panel, the tray).
//! That token arrives in `platform_data` of `Activate`: it is kept here and
//! used on the UI thread, where the window's `wl_surface` is known.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Shared between the D-Bus thread (`Activate`) and the UI thread.
#[derive(Clone, Default)]
pub struct Request {
    raise: Arc<AtomicBool>,
    token: Arc<Mutex<Option<String>>>,
}

impl Request {
    /// Called from the D-Bus thread: the newest token wins.
    pub fn ask(&self, token: Option<String>) {
        if let Ok(mut t) = self.token.lock() {
            if token.is_some() {
                *t = token;
            }
        }
        self.raise.store(true, Ordering::Relaxed);
    }

    /// Called from the UI thread: `None` when nothing was asked, else the
    /// token to use (if any). Clears the request.
    pub fn take(&self) -> Option<Option<String>> {
        if !self.raise.swap(false, Ordering::Relaxed) {
            return None;
        }
        Some(self.token.lock().ok().and_then(|mut t| t.take()))
    }

    /// Forget a request made before the window existed.
    pub fn reset(&self) {
        self.raise.store(false, Ordering::Relaxed);
        if let Ok(mut t) = self.token.lock() {
            *t = None;
        }
    }
}

/// What the window does for one request.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Wayland with a token: `xdg_activation_v1.activate(token, surface)`.
    XdgActivate(String),
    /// Wayland without a token: the compositor may only flag the window.
    Attention,
    /// X11 and others: the window manager honours a focus request.
    Focus,
}

pub fn plan(wayland: bool, token: Option<String>) -> Plan {
    match (wayland, token) {
        (true, Some(t)) if !t.is_empty() => Plan::XdgActivate(t),
        (true, _) => Plan::Attention,
        (false, _) => Plan::Focus,
    }
}

/// Wayland pointers of the window, read through raw-window-handle.
pub fn wayland_handles(
    w: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
) -> Option<(*mut std::ffi::c_void, *mut std::ffi::c_void)> {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    let d = match w.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(d) => d.display.as_ptr(),
        _ => return None,
    };
    let s = match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Wayland(s) => s.surface.as_ptr(),
        _ => return None,
    };
    Some((d, s))
}

#[cfg(target_os = "linux")]
mod wl {
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{
        self, XdgActivationV1,
    };

    struct Nop;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Nop {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<XdgActivationV1, ()> for Nop {
        fn event(
            _: &mut Self,
            _: &XdgActivationV1,
            _: xdg_activation_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    /// `xdg_activation_v1.activate(token, surface)` on the window's own
    /// connection (a private event queue: winit's queue is not touched).
    ///
    /// # Safety
    /// `display` and `surface` must be the live `wl_display` / `wl_surface`
    /// of this process' window (from raw-window-handle, on the UI thread).
    pub unsafe fn activate(
        display: *mut std::ffi::c_void,
        surface: *mut std::ffi::c_void,
        token: &str,
    ) -> Result<(), String> {
        let backend = Backend::from_foreign_display(display.cast());
        let conn = Connection::from_backend(backend);
        let (globals, mut queue) =
            registry_queue_init::<Nop>(&conn).map_err(|e| format!("registry: {e}"))?;
        let qh = queue.handle();
        let act: XdgActivationV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| format!("xdg_activation_v1: {e}"))?;
        let id = ObjectId::from_ptr(WlSurface::interface(), surface.cast())
            .map_err(|e| format!("surface: {e}"))?;
        let surf = WlSurface::from_id(&conn, id).map_err(|e| format!("surface: {e}"))?;
        act.activate(token.to_owned(), &surf);
        act.destroy();
        queue
            .roundtrip(&mut Nop)
            .map(|_| ())
            .map_err(|e| format!("roundtrip: {e}"))
    }
}

/// Carry out one request on the UI thread. Returns what was done, for the log.
pub fn raise(ctx: &eframe::egui::Context, frame: &eframe::Frame, token: Option<String>) -> String {
    use eframe::egui::{UserAttentionType, ViewportCommand};
    let handles = wayland_handles(frame);
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    match plan(handles.is_some(), token) {
        Plan::XdgActivate(t) => {
            #[cfg(target_os = "linux")]
            if let Some((d, s)) = handles {
                // SAFETY: pointers of this window, read just above on the UI thread.
                return match unsafe { wl::activate(d, s, &t) } {
                    Ok(()) => "xdg-activation requested with the caller's token".into(),
                    Err(e) => {
                        ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                            UserAttentionType::Informational,
                        ));
                        format!("xdg-activation failed ({e}): attention requested")
                    }
                };
            }
            let _ = t;
            "no Wayland surface".into()
        }
        Plan::Attention => {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                UserAttentionType::Informational,
            ));
            "no activation token: attention requested (the desktop decides)".into()
        }
        Plan::Focus => {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            "focus requested".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_of_activate_reaches_the_ui_thread() {
        let r = Request::default();
        assert_eq!(r.take(), None);
        r.ask(Some("kwin-7".into()));
        assert_eq!(r.take(), Some(Some("kwin-7".into())));
        assert_eq!(r.take(), None, "one request, one activation");
    }

    #[test]
    fn newest_token_wins_and_a_tokenless_call_keeps_it() {
        let r = Request::default();
        r.ask(Some("old".into()));
        r.ask(Some("new".into()));
        r.ask(None);
        assert_eq!(r.take(), Some(Some("new".into())));
        r.ask(None);
        assert_eq!(r.take(), Some(None));
    }

    #[test]
    fn wayland_uses_the_token_with_xdg_activation() {
        assert_eq!(plan(true, Some("t".into())), Plan::XdgActivate("t".into()));
        assert_eq!(plan(true, Some(String::new())), Plan::Attention);
        assert_eq!(plan(true, None), Plan::Attention);
        assert_eq!(plan(false, Some("t".into())), Plan::Focus);
    }

    #[test]
    fn reset_forgets_an_early_request() {
        let r = Request::default();
        r.ask(Some("t".into()));
        r.reset();
        assert_eq!(r.take(), None);
    }
}
