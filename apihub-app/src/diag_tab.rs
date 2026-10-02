//! The "Diag" tab: checks of the installed components, run on a worker
//! thread (extracted from `main.rs`, #62).

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;

use crate::i18n::{tr, trf};
use crate::view::Palette;

#[derive(Clone)]
struct DiagResult {
    label: String,
    ok: bool,
    detail: String,
}

pub struct DiagTab {
    results: Arc<Mutex<Vec<DiagResult>>>,
    running: Arc<AtomicBool>,
}

impl DiagTab {
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    fn run(&mut self) {
        // Guard against concurrent runs
        if self.running.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Ok(mut r) = self.results.lock() {
            r.clear();
        }

        let results = self.results.clone();
        let running = self.running.clone();

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

            // rssi-helper caps
            let rssi_ok = std::path::Path::new("/usr/lib/apple-kb-monitor/rssi-helper").exists();
            out.push(DiagResult {
                label: tr("RSSI helper").into(),
                ok: rssi_ok,
                detail: if rssi_ok {
                    tr("rssi-helper installed (needs CAP_NET_ADMIN)").into()
                } else {
                    tr("NOT FOUND").into()
                },
            });

            // Store results (the guard clears the running flag on drop)
            if let Ok(mut r) = results.lock() {
                *r = out;
            }
        });
    }

    pub fn show(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let is_running = self.running.load(std::sync::atomic::Ordering::Relaxed);

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr("System Diagnostics"))
                    .strong()
                    .size(18.0),
            );
            if is_running {
                ui.label(
                    egui::RichText::new(tr("Running..."))
                        .size(16.0)
                        .color(palette.warn),
                );
            } else if ui
                .button(
                    egui::RichText::new(tr("Run Full Check"))
                        .size(16.0)
                        .strong(),
                )
                .clicked()
            {
                self.run();
            }
        });
        ui.separator();

        let results = self.results.lock().map(|r| r.clone()).unwrap_or_default();

        if results.is_empty() {
            if is_running {
                ui.label(
                    egui::RichText::new(tr("Diagnostics in progress..."))
                        .weak()
                        .size(16.0),
                );
            } else {
                ui.label(
                    egui::RichText::new(tr("Press 'Run Full Check' to scan all components."))
                        .weak()
                        .size(16.0),
                );
            }
            return;
        }

        let total = results.len();
        let ok_count = results.iter().filter(|r| r.ok).count();
        let fail_count = total - ok_count;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(trf("{}/{} passed", &[&ok_count, &total]))
                    .strong()
                    .size(18.0)
                    .color(if fail_count == 0 {
                        palette.good
                    } else {
                        palette.warn
                    }),
            );
            if fail_count > 0 {
                ui.label(
                    egui::RichText::new(trf("  {} issues", &[&fail_count]))
                        .size(16.0)
                        .color(palette.bad),
                );
            }
        });

        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("diag")
                    .num_columns(3)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        for r in &results {
                            let (icon, c) = if r.ok {
                                (tr("OK"), palette.good)
                            } else {
                                (tr("FAIL"), palette.bad)
                            };
                            ui.label(egui::RichText::new(icon).size(16.0).strong().color(c));
                            ui.label(egui::RichText::new(&r.label).strong().size(16.0));
                            // Long details wrap instead of running off the window (#195).
                            ui.add(
                                egui::Label::new(egui::RichText::new(&r.detail).weak().size(16.0))
                                    .wrap(),
                            );
                            ui.end_row();
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
