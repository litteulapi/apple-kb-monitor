//! DIAG tab: checks of the installed components, run on a worker thread
//! (extracted from `main.rs`, #62) and shown as a terminal log.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;

use crate::i18n::{tr, trf};
use crate::theme::{self, Theme};

#[derive(Clone)]
struct DiagResult {
    label: String,
    ok: bool,
    detail: String,
}

pub struct DiagTab {
    results: Arc<Mutex<Vec<DiagResult>>>,
    running: Arc<AtomicBool>,
    /// Why the daemon does not measure the signal (`radio.rssi_error` of
    /// the last snapshot, #269), set by the window before each run.
    pub signal: Option<akm_core::rssi::RssiIssue>,
}

impl DiagTab {
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            signal: None,
        }
    }

    /// Start the checks (button, F5); ignored while they are running.
    pub fn run(&mut self) {
        // Guard against concurrent runs
        if self.running.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Ok(mut r) = self.results.lock() {
            r.clear();
        }

        let results = self.results.clone();
        let running = self.running.clone();
        let signal = self.signal.clone();

        /// Clears the "running" flag even if the diagnostics thread panics.
        struct RunningGuard(Arc<AtomicBool>);
        impl Drop for RunningGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Relaxed);
            }
        }

        thread::spawn(move || {
            let _guard = RunningGuard(running);
            let mut out: Vec<DiagResult> = Vec::new();

            let checks: Vec<(&str, Vec<String>, &str)> = vec![
                (
                    "apple-kb-monitord",
                    vec!["--version".into()],
                    tr("Monitor daemon binary"),
                ),
                ("bluetoothctl", vec!["--version".into()], tr("BlueZ CLI")),
            ];

            for (bin, args, desc) in &checks {
                let result = crate::diag::run_bounded(
                    Command::new(bin).args(args.iter().map(|s| s.as_str())),
                    crate::diag::COMMAND_TIMEOUT,
                );
                match result {
                    Ok(o) if o.status.success() => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let first = stdout.lines().next().unwrap_or("OK").trim();
                        out.push(DiagResult {
                            label: desc.to_string(),
                            ok: true,
                            detail: format!(
                                "{}: {}",
                                bin,
                                if first.is_empty() { "OK" } else { first }
                            ),
                        });
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        if err.contains("Usage") || err.contains("usage") {
                            out.push(DiagResult {
                                label: desc.to_string(),
                                ok: true,
                                detail: trf("{}: installed", &[bin]),
                            });
                        } else {
                            out.push(DiagResult {
                                label: desc.to_string(),
                                ok: false,
                                detail: trf("{}: exit {}", &[bin, &o.status.code().unwrap_or(-1)]),
                            });
                        }
                    }
                    Err(crate::diag::RunError::Timeout) => {
                        out.push(DiagResult {
                            label: desc.to_string(),
                            ok: false,
                            detail: trf(
                                "{}: no answer within {} s (killed)",
                                &[bin, &crate::diag::COMMAND_TIMEOUT.as_secs()],
                            ),
                        });
                    }
                    Err(crate::diag::RunError::Spawn) => {
                        out.push(DiagResult {
                            label: desc.to_string(),
                            ok: false,
                            detail: trf("{}: NOT FOUND", &[bin]),
                        });
                    }
                }
            }

            // Daemon: owner of the keyboard, reached over the session bus
            let active = crate::diag::run_bounded(
                Command::new("systemctl").args([
                    "--user",
                    "is-active",
                    "apple-kb-monitord.service",
                ]),
                crate::diag::COMMAND_TIMEOUT,
            )
            .map(|o| o.status.success())
            .unwrap_or(false);
            out.push(DiagResult {
                label: "apple-kb-monitord.service".into(),
                ok: active,
                detail: if active {
                    tr("active (running)").into()
                } else {
                    tr("inactive / not found").into()
                },
            });
            let on_bus = crate::instance::bounded(crate::diag::COMMAND_TIMEOUT, false, || {
                zbus::blocking::Connection::session()
                    .map(|c| apple_kb_monitord::client::daemon_present(&c))
                    .unwrap_or(false)
            });
            out.push(DiagResult {
                label: "D-Bus com.agenceapi.AppleKbMonitor1".into(),
                ok: on_bus,
                detail: if on_bus {
                    tr("daemon reachable (this window is a client)").into()
                } else {
                    tr("daemon absent: this app reads the keyboard itself").into()
                },
            });

            // Apple keyboard hidraw node: present AND readable by this user
            // (udev rule uses TAG+="uaccess", no group membership needed).
            let (hid_ok, hid_detail) = match crate::keyboard::find_apple_hidraw() {
                None => (
                    false,
                    tr("no Apple hidraw device found (keyboard off or not paired?)").to_string(),
                ),
                Some(path) => match std::fs::File::open(&path) {
                    Ok(_) => (true, trf("{}: readable", &[&path])),
                    Err(e) => (
                        false,
                        trf("{}: {} — check the udev uaccess rule", &[&path, &e]),
                    ),
                },
            };
            out.push(DiagResult {
                label: tr("hidraw readable").into(),
                ok: hid_ok,
                detail: hid_detail,
            });

            // Key mapping (#247): udev hwdb written by `akmctl keymap apply`;
            // none = kernel mapping (the default). keyd is optional (#246).
            let hwdb = akm_core::keymap::HWDB_PATH;
            let (km_ok, mut km_detail) = match akm_core::keymap::read_installed(std::path::Path::new(hwdb)) {
                Ok(r) if r.is_empty() => (true, tr("kernel mapping, no hwdb installed (optional: akmctl keymap; check: akmctl keys --check)").to_string()),
                Ok(r) => (true, trf("{}: {} model(s) remapped (akmctl keys --check)", &[&hwdb, &r.len()])),
                Err(e) => (false, trf("{} - reinstall: akmctl keymap apply, or remove: akmctl keymap reset", &[&e])),
            };
            let keyd_active = crate::diag::run_bounded(
                Command::new("systemctl").args(["is-active", "--quiet", "keyd.service"]),
                crate::diag::COMMAND_TIMEOUT,
            )
            .map(|o| o.status.success())
            .unwrap_or(false);
            if keyd_active && std::path::Path::new("/etc/keyd/apple-keyboard.conf").exists() {
                km_detail = trf("{} · keyd also active with /etc/keyd/apple-keyboard.conf (optional, see KEYD.md)", &[&km_detail]);
            }
            out.push(DiagResult {
                label: tr("Key mapping").into(),
                ok: km_ok,
                detail: km_detail,
            });

            // udev rules
            let udev_ok =
                std::path::Path::new("/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules").exists();
            out.push(DiagResult {
                label: tr("udev rules").into(),
                ok: udev_ok,
                detail: if udev_ok {
                    tr("70-apple-kb-hidraw.rules installed").into()
                } else {
                    tr("NOT FOUND").into()
                },
            });

            // hid_apple fnmode: applied value (sysfs) vs configured (modprobe.d)
            let (fn_ok, fn_detail) = crate::fnmode_diag::diagnose();
            out.push(DiagResult {
                label: tr("hid_apple fnmode").into(),
                ok: fn_ok,
                detail: fn_detail,
            });

            // Signal (#269): the daemon's last real attempt first, else can
            // this account run the helper at all (`root:akm 0750`).
            out.push(signal_row(&HelperProbe::system(), signal.as_ref()));

            // Store results (the guard clears the running flag on drop)
            if let Ok(mut r) = results.lock() {
                *r = out;
            }
        });
    }

    /// The tab: a terminal log, one entry per check, each failed entry
    /// carrying its fix; one gauge block per check on top.
    pub fn show(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let is_running = self.running.load(std::sync::atomic::Ordering::Relaxed);
        let results = self.results.lock().map(|r| r.clone()).unwrap_or_default();
        theme::scroll_body(ui, crate::shell::Tab::Diag.label(), |ui| {
            th.panel(ui, tr("System Diagnostics"), 0.0, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if theme::action(ui, tr("Run Full Check"), !is_running).clicked() {
                        self.run();
                    }
                    if is_running {
                        theme::text(ui, tr("Running..."), theme::BODY, theme::AMBER);
                    }
                });
                if results.is_empty() {
                    let msg = if is_running {
                        tr("Diagnostics in progress...")
                    } else {
                        tr("Press 'Run Full Check' to scan all components.")
                    };
                    theme::text(ui, &format!("> {msg} _"), theme::BODY, theme::GREEN_MID);
                    return;
                }
                let total = results.len();
                let ok_count = results.iter().filter(|r| r.ok).count();
                let fail_count = total - ok_count;
                let color = if fail_count == 0 {
                    theme::PHOSPHOR
                } else {
                    theme::AMBER
                };
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    th.glow_label(
                        ui,
                        &theme::caps(&trf("{}/{} passed", &[&ok_count, &total])),
                        theme::VALUE,
                        color,
                    );
                    if fail_count > 0 {
                        th.glow_label(
                            ui,
                            &theme::caps(&trf("  {} issues", &[&fail_count])),
                            theme::VALUE,
                            theme::RED,
                        );
                    }
                });
                // One block per check, in the order of the log below.
                theme::segments(ui, 16.0, total, |i| {
                    Some(if results[i].ok {
                        theme::PHOSPHOR
                    } else {
                        theme::RED
                    })
                });
            });
            if results.is_empty() {
                return;
            }
            ui.add_space(theme::GAP);
            th.panel(ui, tr("Log"), 0.0, |ui| {
                for (i, r) in results.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(2.0);
                    }
                    entry(ui, th, r);
                }
            });
        });
    }
}

const RSSI_HELPER: &str = "/usr/lib/apple-kb-monitor/rssi-helper";

/// What the signal row looks at (a fixture in tests).
struct HelperProbe {
    exists: bool,
    executable: bool,
    listed_in_akm: bool,
}

impl HelperProbe {
    fn system() -> Self {
        let c = std::ffi::CString::new(RSSI_HELPER).unwrap_or_default();
        Self {
            exists: std::path::Path::new(RSSI_HELPER).exists(),
            // SAFETY: access(2) on a NUL-terminated constant path, no effect.
            executable: unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0,
            listed_in_akm: akm_core::rssi::user_listed_in_akm(),
        }
    }
}

/// The "signal" row of DIAG: OK only when the signal can really be measured.
fn signal_row(p: &HelperProbe, daemon: Option<&akm_core::rssi::RssiIssue>) -> DiagResult {
    use akm_core::rssi;
    let code = if !p.exists {
        Some(rssi::CODE_HELPER_MISSING.to_string())
    } else if let Some(i) = daemon {
        Some(i.code.clone())
    } else if !p.executable {
        Some(rssi::classify(&rssi::RssiError::Denied, || p.listed_in_akm).code)
    } else {
        None
    };
    match code {
        None => DiagResult {
            label: tr("Signal measure").into(),
            ok: true,
            detail: tr("rssi-helper runnable by this account (group akm)").into(),
        },
        Some(c) => {
            let (why, fix) = rssi::explain(&c, crate::i18n::is_french());
            DiagResult {
                label: tr("Signal measure").into(),
                ok: false,
                detail: format!("{}. {} {fix}", crate::view::capitalize(why), tr("Fix:")),
            }
        }
    }
}

/// Width of the `[ OK ]` / `[FAIL]` column, sized for the longest tag.
fn tag_column() -> f32 {
    let longest = [tr("OK"), tr("FAIL")]
        .iter()
        .map(|t| t.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    (longest + 3) as f32 * theme::ADVANCE * theme::BODY
}

/// `[ OK ]` or `[FAIL]`, padded to the same width.
fn tag(ok: bool) -> String {
    let word = if ok { tr("OK") } else { tr("FAIL") };
    let width = [tr("OK"), tr("FAIL")]
        .iter()
        .map(|t| t.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    format!("[{word:^width$}]")
}

/// One log entry: the tag, the name of the check, then its detail (which
/// holds the fix when the check failed) wrapped under the name.
fn entry(ui: &mut egui::Ui, th: &Theme, r: &DiagResult) {
    let color = if r.ok { theme::PHOSPHOR } else { theme::RED };
    let avail = ui.available_width();
    let col = tag_column();
    let text_w = (avail - col).max(60.0);
    let p = ui.painter();
    let label = p.layout(
        theme::caps(&r.label),
        theme::font(theme::BODY),
        theme::PHOSPHOR,
        text_w,
    );
    // A failed check is read in full phosphor: its detail is the fix.
    let detail_ink = if r.ok {
        theme::GREEN_MID
    } else {
        theme::PHOSPHOR
    };
    let detail = p.layout(
        theme::glyphs(&r.detail).into_owned(),
        theme::font(theme::BODY),
        detail_ink,
        text_w,
    );
    let h = label.size().y + detail.size().y + 2.0;
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(avail, h), egui::Sense::hover());
    let p = ui.painter();
    th.glow(
        p,
        rect.left_top(),
        egui::Align2::LEFT_TOP,
        &tag(r.ok),
        theme::BODY,
        color,
    );
    let x = rect.left() + col;
    let ly = label.size().y;
    p.galley(egui::Pos2::new(x, rect.top()), label, theme::PHOSPHOR);
    p.galley(egui::Pos2::new(x, rect.top() + ly), detail, detail_ink);
    let said = format!("{} {}: {}", tag(r.ok), r.label, r.detail);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(exists: bool, executable: bool, listed_in_akm: bool) -> HelperProbe {
        HelperProbe {
            exists,
            executable,
            listed_in_akm,
        }
    }

    /// #269: the helper exists but this account may not run it — the very
    /// failure of the user — is a FAILED row with the command to type.
    #[test]
    fn signal_row_detects_a_helper_this_account_cannot_run() {
        let r = signal_row(&probe(true, false, false), None);
        assert!(!r.ok);
        assert!(r.detail.contains("not in the group akm"), "{}", r.detail);
        assert!(r.detail.contains("sudo usermod -aG akm $USER"));
        let r = signal_row(&probe(true, false, true), None);
        assert!(!r.ok);
        assert!(r.detail.contains("restart the computer"), "{}", r.detail);
        assert!(signal_row(&probe(true, true, false), None).ok);
        let r = signal_row(&probe(false, false, false), None);
        assert!(!r.ok && r.detail.contains("not installed"));
    }

    /// The daemon's own failure wins: its groups are not the window's.
    #[test]
    fn signal_row_trusts_the_daemon_reason() {
        let issue = akm_core::rssi::classify(&akm_core::rssi::RssiError::Denied, || true);
        let r = signal_row(&probe(true, true, true), Some(&issue));
        assert!(!r.ok, "the window may run it, the daemon may not");
        assert!(r.detail.contains("service started before"), "{}", r.detail);
    }

    #[test]
    fn tags_have_one_width() {
        assert_eq!(tag(true), "[ OK ]");
        assert_eq!(tag(false), "[FAIL]");
        assert_eq!(tag(true).chars().count(), tag(false).chars().count());
        assert!(tag_column() >= 7.0 * theme::ADVANCE * theme::BODY);
    }

    #[test]
    fn diag_results_survive_a_panicking_writer() {
        let r: Arc<Mutex<Vec<DiagResult>>> = Arc::new(Mutex::new(Vec::new()));
        let r2 = r.clone();
        let _ = thread::spawn(move || {
            let _g = r2.lock().unwrap();
            panic!("boom");
        })
        .join();
        assert!(r.lock().is_err());
        r.clear_poison();
        assert!(r.lock().is_ok());
    }
}
