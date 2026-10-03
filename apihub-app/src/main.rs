mod actions;
mod activation;
mod cli;
mod diag;
mod fn_toggle;
mod fnmode_diag;
mod framestats;
mod heartbeat;
mod history_chart;
mod history_view;
mod i18n;
mod instance;
mod keyboard;
mod krunner;
mod rename;
mod settings;
mod shell;
mod source;
mod tab_data;
mod tab_diag;
mod tab_keys;
mod tab_radio;
mod tab_stat;
#[cfg(test)]
mod testbus;
mod theme;
mod view;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use akm_core::Watch;
use eframe::egui;

use shell::{Nav, Tab};
use theme::Theme;

/// Latest keyboard state, published by the acquisition actor.
type State = Arc<Watch>;

fn unix_now() -> u64 {
    akm_core::history::Clock::now(&akm_core::history::SystemClock)
}

// ── App ─────────────────────────────────────────────────────────────────────

struct ApiHubApp {
    state: State,
    // Daemon or local fallback (alert of the shell).
    feed: source::FeedCell,
    // Arrivals of the daemon seen so far: a new one reloads the history (#272).
    daemon_arrivals: u64,
    tab: Tab,
    theme: Theme,
    style_initialized: bool,
    diag: tab_diag::DiagTab,
    // Set by a second launch / D-Bus Activate: bring the window to front (#270)
    raise: activation::Request,
    // Battery history graph: loaded by a worker thread, only read here.
    history: history_view::Loader,
    // Period of the history chart (#96): 24 h, 7, 30 or 90 days.
    history_range: view::Range,
    // Rename field (#141)
    rename: tab_data::Rename,
    // Reconnect request and Fn mode, through the daemon, off this thread.
    link: actions::Job,
    fnmode: actions::FnMode,
    frame_stats: framestats::FrameStats,
    // UI heartbeat (#240): ticked from update(), written by a small thread.
    heartbeat: Option<heartbeat::Heartbeat>,
}

impl ApiHubApp {
    /// Create the GUI app. Does NOT spawn the acquisition — it is owned by
    /// main() and shared through the `Watch`.
    fn new(
        cc: &eframe::CreationContext<'_>,
        state: State,
        feed: source::FeedCell,
        raise: activation::Request,
        ui: settings::UiSettings,
    ) -> Self {
        // Battery history: loaded off the UI thread (D-Bus + disk, #230).
        let history = history_view::Loader::new();
        history_chart::reload(&cc.egui_ctx, &history);
        let fnmode = actions::FnMode::default();
        let ctx = cc.egui_ctx.clone();
        fnmode.refresh(move || ctx.request_repaint());

        Self {
            state,
            feed,
            daemon_arrivals: 0,
            tab: Tab::Stat,
            theme: Theme {
                crt: ui.crt_effects,
            },
            style_initialized: false,
            diag: tab_diag::DiagTab::new(),
            raise,
            history,
            history_range: view::Range::default(),
            rename: tab_data::Rename::new(),
            link: actions::Job::default(),
            fnmode,
            frame_stats: framestats::FrameStats::from_env(),
            heartbeat: heartbeat::Heartbeat::from_env(),
        }
    }
}

impl eframe::App for ApiHubApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let t0 = std::time::Instant::now();
        // A second launch or Activate asked for the window: use its
        // activation token on this thread, where the surface is known (#270).
        if let Some(token) = self.raise.take() {
            eprintln!(
                "[apihub] activate: {}",
                activation::raise(ctx, frame, token)
            );
        }
        self.update_ui(ctx);
        let took = t0.elapsed();
        self.frame_stats.record(took, frame.info().cpu_usage);
        if let Some(hb) = self.heartbeat.as_mut() {
            hb.tick(took);
        }
    }

    /// The backdrop is painted by the theme: clear to the CRT background.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }
}

impl ApiHubApp {
    fn update_ui(&mut self, ctx: &egui::Context) {
        // Font, palette and style of the terminal — once only.
        if !self.style_initialized {
            theme::install(ctx);
            self.style_initialized = true;
        }

        // Next repaint in 1 s (also minimised/hidden): drives the UI heartbeat
        // (#240); egui sleeps until then or until user interaction. The CRT
        // effects are static: they never ask for a frame.
        ctx.request_repaint_after(heartbeat::PERIOD);

        let snap = self.state.get();
        let feed = self.feed.get();
        // History reloaded by itself (#272): daemon back, or stale on DATA.
        shell::set_daemon_online(ctx, feed != view::Feed::Local);
        let arrivals = self.feed.arrivals();
        let came_back = arrivals != self.daemon_arrivals;
        self.daemon_arrivals = arrivals;
        let data = self.history.data();
        let age = data.loaded_at.map(|t| t.elapsed());
        if !data.loading && history_view::reload_due(came_back, self.tab == Tab::Data, age) {
            history_chart::reload(ctx, &self.history);
        }
        let now = unix_now();
        self.navigate(ctx);

        self.theme.backdrop(ctx);
        let side = theme::GUTTER as i8;
        let frame = |top: i8, bottom: i8| {
            egui::Frame::new().inner_margin(egui::Margin {
                left: side,
                right: side,
                top,
                bottom,
            })
        };
        egui::TopBottomPanel::top("header")
            .frame(frame(12, 4))
            .show_separator_line(false)
            .show(ctx, |ui| {
                centred(ui, |ui| {
                    shell::header(ui, &self.theme, &snap);
                    if let Some(tab) = shell::tab_bar(ui, &self.theme, self.tab) {
                        self.tab = tab;
                    }
                    shell::alerts(ui, &self.theme, &snap, feed);
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(frame(2, 10))
            .show_separator_line(false)
            .show(ctx, |ui| centred(ui, |ui| shell::status_bar(ui, &snap)));
        egui::CentralPanel::default()
            .frame(frame(4, 4))
            .show(ctx, |ui| {
                centred(ui, |ui| self.body(ui, &snap, now));
            });
        self.theme.overlay(ctx);
    }

    /// The current tab.
    fn body(&mut self, ui: &mut egui::Ui, snap: &akm_core::Snapshot, now: u64) {
        match self.tab {
            Tab::Stat => {
                let asked = tab_stat::show(ui, &self.theme, snap, now, &self.link, &self.fnmode);
                if asked == Some(tab_stat::Request::Rename) {
                    self.tab = Tab::Data;
                    self.rename.focus = true;
                }
            }
            Tab::Radio => tab_radio::show(ui, &self.theme, snap, now, &self.link),
            Tab::Keys => tab_keys::show(ui, &self.theme, &self.fnmode),
            Tab::Data => tab_data::show(
                ui,
                &self.theme,
                snap,
                now,
                &self.history,
                &mut self.history_range,
                &mut self.rename,
            ),
            Tab::Diag => {
                self.diag.signal = signal_issue(snap);
                self.diag.last_error = snap.last_error.clone();
                self.diag.show(ui, &self.theme)
            }
        }
    }

    /// Keyboard navigation: digits and arrows for the tabs, arrows and page
    /// keys to scroll, Escape, F5 (see `shell::nav_for`).
    fn navigate(&mut self, ctx: &egui::Context) {
        for nav in shell::navigation(ctx) {
            match nav {
                Nav::Go(tab) => self.tab = tab,
                Nav::Step(d) => self.tab = self.tab.step(d),
                Nav::Scroll(dy) => theme::request_scroll(ctx, dy),
                Nav::Back => match ctx.memory(|m| m.focused()) {
                    Some(id) => ctx.memory_mut(|m| m.surrender_focus(id)),
                    None => self.tab = Tab::Stat,
                },
                Nav::Reload => self.reload(ctx),
            }
        }
    }

    /// F5: reload what the current tab shows.
    fn reload(&mut self, ctx: &egui::Context) {
        let repaint = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        match self.tab {
            Tab::Stat | Tab::Radio => self.fnmode.refresh(repaint),
            Tab::Keys => {
                self.fnmode.refresh(repaint);
                tab_keys::reload(ctx);
            }
            Tab::Data => history_chart::reload(ctx, &self.history),
            Tab::Diag => {
                let snap = self.state.get();
                self.diag.signal = signal_issue(&snap);
                self.diag.last_error = snap.last_error.clone();
                self.diag.run()
            }
        }
    }
}

/// Very wide windows: header, tabs, body and status bar share one centred
/// column of at most `MAX_CONTENT`, never stretched apart (#282).
fn centred<R>(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let spare = (ui.available_width() - theme::MAX_CONTENT).max(0.0) / 2.0;
    let rect = ui.max_rect().shrink2(egui::Vec2::new(spare, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), f)
        .inner
}

/// Why the daemon does not measure the signal of the connected keyboard (#269).
fn signal_issue(snap: &akm_core::Snapshot) -> Option<akm_core::rssi::RssiIssue> {
    snap.connected
        .then(|| snap.keyboard.as_ref()?.radio.rssi_error.clone())
        .flatten()
}

// ── Entrypoint ──────────────────────────────────────────────────────────────

/// Open the window and block until it is closed. Returns false when it
/// could not be opened (no display / GPU). Called **once** per process:
/// winit does not support a second event loop run reliably (#226).
fn open_window(
    state: &State,
    feed: &source::FeedCell,
    raise: &activation::Request,
    open: &Arc<AtomicBool>,
) -> bool {
    // `[ui]` of config.toml, read once before the window exists.
    let ui = settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(crate::i18n::tr("Apple Keyboard Monitor"))
            .with_app_id(instance::APP_ID)
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([420.0, 400.0]),
        // Never block in eglSwapBuffers: with vsync on, Mesa waits for a
        // Wayland frame callback that a minimized or hidden window never
        // gets, the main thread stops answering pings ("Not responding") and
        // ignores Activate/Quit (#231). Repaints are timer-driven (1 s) or
        // input-driven anyway, so there is nothing to tear.
        vsync: false,
        // Return to main() on close, which then ends the process (#226).
        run_and_return: true,
        ..Default::default()
    };
    let (st, fd, sw) = (state.clone(), feed.clone(), raise.clone());
    raise.reset();
    open.store(true, Ordering::Relaxed);
    let r = eframe::run_native(
        instance::APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(ApiHubApp::new(cc, st, fd, sw, ui)))),
    );
    open.store(false, Ordering::Relaxed);
    if let Err(ref e) = r {
        eprintln!("[apihub] cannot open window: {}", e);
    }
    r.is_ok()
}

fn main() {
    // Windowless modes, thin D-Bus clients of the daemon: the command of the
    // global shortcut (#99) and the KRunner runner (#98).
    // Every argument is handled here, before the single-instance claim:
    // `--help` must never activate a running window (#271).
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        cli::Cli::Window => {}
        cli::Cli::ToggleFn => std::process::exit(fn_toggle::run()),
        cli::Cli::Krunner => std::process::exit(krunner::run()),
        cli::Cli::Help => {
            print!("{}", cli::help());
            return;
        }
        cli::Cli::Version => {
            println!("{}", cli::version());
            return;
        }
        cli::Cli::Bad(a) => {
            eprintln!("apihub-app: {}", i18n::trf("unknown argument: {}", &[&a]));
            eprintln!("{}", i18n::tr("Try 'apihub-app --help'."));
            std::process::exit(2);
        }
    }
    // One window = one process (#226): `apihub-app` or D-Bus
    // `org.freedesktop.Application` Activate opens the window; a second launch
    // raises it and exits; closing the window ends the process, which frees
    // the D-Bus name. Nothing ever reopens a window by itself. The tray icon
    // belongs to the daemon alone (#62): this process never registers one.
    let window_open = Arc::new(AtomicBool::new(false));
    let raise = activation::Request::default();
    let activate = {
        let raise = raise.clone();
        // The window is (being) opened by this process: hand the token to
        // the UI thread, which activates its surface with it (#270). Never
        // `set_var` from this D-Bus thread: undefined behaviour (#197).
        move |token: Option<String>| raise.ask(token)
    };
    let conn = match instance::claim(activate) {
        instance::Claim::Existing => {
            eprintln!("[apihub] already running: activation sent to the open window");
            return;
        }
        instance::Claim::Unreachable => std::process::exit(1),
        instance::Claim::Primary(c) => Some(c),
        instance::Claim::NoBus => None,
    };

    let state: State = Arc::new(Watch::new());
    let src = source::spawn(state.clone());

    let shown = open_window(&state, &src.feed(), &raise, &window_open);
    eprintln!("[apihub] window closed: exiting");
    // Free the name first so that a new launch becomes the window at once.
    drop(conn);
    src.stop();
    std::process::exit(if shown { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use akm_core::Snapshot;

    #[test]
    fn tooltip_shows_na_without_sources() {
        let t = Snapshot::default().tooltip_text();
        assert!(t.contains("n/a"));
        assert!(!t.contains("0%"));
    }
}
