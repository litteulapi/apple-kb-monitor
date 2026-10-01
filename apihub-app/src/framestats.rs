//! Frame timing probe (#230): `AKM_FRAME_STATS=<file>` appends one line per
//! second of activity with the frame count and the slowest frame, so a
//! blocked UI thread is measured instead of guessed. Off by default: no cost.

use std::io::Write;
use std::time::{Duration, Instant};

pub struct FrameStats {
    out: Option<std::fs::File>,
    window_start: Instant,
    frames: u64,
    max_update: Duration,
    max_cpu: f32,
    total_frames: u64,
}

impl FrameStats {
    pub fn from_env() -> Self {
        let out = std::env::var_os("AKM_FRAME_STATS")
            .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
        Self { out, window_start: Instant::now(), frames: 0, max_update: Duration::ZERO, max_cpu: 0.0, total_frames: 0 }
    }

    /// `update`: time spent in `App::update`; `cpu`: eframe's measure of the
    /// previous frame (update + tessellation + paint, vsync wait excluded).
    pub fn record(&mut self, update: Duration, cpu: Option<f32>) {
        let Some(out) = self.out.as_mut() else { return };
        self.frames += 1;
        self.total_frames += 1;
        self.max_update = self.max_update.max(update);
        self.max_cpu = self.max_cpu.max(cpu.unwrap_or(0.0));
        if self.window_start.elapsed() >= Duration::from_secs(1) {
            let _ = writeln!(
                out,
                "frames={} total={} max_update_ms={:.2} max_cpu_ms={:.2}",
                self.frames,
                self.total_frames,
                self.max_update.as_secs_f64() * 1e3,
                f64::from(self.max_cpu) * 1e3
            );
            self.window_start = Instant::now();
            self.frames = 0;
            self.max_update = Duration::ZERO;
            self.max_cpu = 0.0;
        }
    }
}
