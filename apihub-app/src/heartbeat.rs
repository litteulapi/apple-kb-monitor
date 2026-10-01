//! UI heartbeat (#240), contract of docs/QA-AUTOMATIQUE.md §3.2.
//!
//! `$XDG_RUNTIME_DIR/apple-kb-monitor/ui-heartbeat.json` is refreshed by the
//! UI thread at the end of real frames (`tick`, called from `App::update`),
//! at most once per second. The UI thread never touches the disk: it hands a
//! ready line to a small writer thread through a channel holding at most one
//! pending write (a full channel drops the new line). The writer thread has no
//! clock and no timer of its own: if the UI stops ticking, the file goes stale,
//! which is exactly what `akmctl selftest` looks for. Removed on clean exit.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const FILE_NAME: &str = "ui-heartbeat.json";
const TMP_NAME: &str = ".ui-heartbeat.json.tmp";
pub const PERIOD: Duration = Duration::from_secs(1);

/// One heartbeat line (without the trailing newline).
pub fn render(pid: u32, ts_ms: u64, frames: u64, frame_max_ms: f64, frame_last_ms: f64) -> String {
    format!(
        "{{\"pid\":{pid},\"ts_ms\":{ts_ms},\"frames\":{frames},\"frame_max_ms\":{:.1},\"frame_last_ms\":{:.1},\"version\":\"{}\"}}",
        frame_max_ms,
        frame_last_ms,
        env!("CARGO_PKG_VERSION")
    )
}

/// Rate limiter and frame statistics; pure (the caller supplies the clock).
pub struct Pacer {
    last_write: Option<Instant>,
    frames: u64,
    max: Duration,
}

impl Pacer {
    pub fn new() -> Self {
        Self { last_write: None, frames: 0, max: Duration::ZERO }
    }

    /// Records a finished frame. Returns `(frames, max, last)` when a line is
    /// due: the first frame, then at most one per `PERIOD`.
    pub fn frame(&mut self, now: Instant, took: Duration) -> Option<(u64, Duration, Duration)> {
        self.frames += 1;
        self.max = self.max.max(took);
        let due = self.last_write.is_none_or(|t| now.saturating_duration_since(t) >= PERIOD);
        if !due {
            return None;
        }
        self.last_write = Some(now);
        let max = std::mem::replace(&mut self.max, Duration::ZERO);
        Some((self.frames, max, took))
    }
}

/// Private directory (0700, ours, not a symlink); `None` if it cannot be made so.
pub fn ensure_dir(dir: &Path) -> Option<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let md = std::fs::symlink_metadata(dir).ok()?;
    // SAFETY: geteuid has no preconditions.
    if !md.is_dir() || md.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    if md.mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(())
}

/// Temporary file (0600) then `rename()`: readers never see a partial line.
pub fn write_atomic(dir: &Path, line: &str) -> std::io::Result<()> {
    let tmp = dir.join(TMP_NAME);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    drop(f);
    std::fs::rename(&tmp, dir.join(FILE_NAME))
}

pub struct Heartbeat {
    dir: PathBuf,
    pacer: Pacer,
    tx: Option<SyncSender<String>>,
    writer: Option<JoinHandle<()>>,
}

impl Heartbeat {
    /// `$XDG_RUNTIME_DIR/apple-kb-monitor`; disabled without a runtime dir.
    pub fn from_env() -> Option<Self> {
        let run = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty())?;
        Self::start(Path::new(&run).join("apple-kb-monitor"))
    }

    pub fn start(dir: PathBuf) -> Option<Self> {
        ensure_dir(&dir)?;
        let (tx, rx) = sync_channel::<String>(1);
        let d = dir.clone();
        let writer = std::thread::Builder::new()
            .name("ui-heartbeat".into())
            .spawn(move || {
                // Ends when the sender is dropped; writes only what it is handed.
                while let Ok(line) = rx.recv() {
                    let _ = write_atomic(&d, &line);
                }
            })
            .ok()?;
        Some(Self { dir, pacer: Pacer::new(), tx: Some(tx), writer: Some(writer) })
    }

    /// End of a UI frame. Never blocks: a pending write is not queued twice.
    pub fn tick(&mut self, took: Duration) {
        let Some((frames, max, last)) = self.pacer.frame(Instant::now(), took) else { return };
        let ts_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let line = render(std::process::id(), ts_ms, frames, max.as_secs_f64() * 1e3, last.as_secs_f64() * 1e3);
        if let Some(tx) = &self.tx {
            match tx.try_send(line) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => self.tx = None,
            }
        }
    }
}

impl Drop for Heartbeat {
    /// Clean close: let the writer finish, then remove the file.
    fn drop(&mut self) {
        self.tx = None;
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        let _ = std::fs::remove_file(self.dir.join(FILE_NAME));
        let _ = std::fs::remove_file(self.dir.join(TMP_NAME));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("akm-hb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn wait_for(p: &Path) -> String {
        for _ in 0..200 {
            if let Ok(s) = std::fs::read_to_string(p) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no file {}", p.display());
    }

    #[test]
    fn line_matches_the_contract() {
        let l = render(1234, 1790860000123, 5321, 18.44, 6.06);
        let v: serde_json::Value = serde_json::from_str(&l).unwrap();
        assert_eq!(v["pid"], 1234);
        assert_eq!(v["ts_ms"], 1790860000123u64);
        assert_eq!(v["frames"], 5321);
        assert_eq!(v["frame_max_ms"], 18.4);
        assert_eq!(v["frame_last_ms"], 6.1);
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert!(!l.contains('\n'));
    }

    #[test]
    fn pacer_limits_to_one_line_per_second_and_keeps_the_max() {
        let mut p = Pacer::new();
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        assert!(p.frame(t0, ms(5)).is_some(), "first frame writes at once");
        for i in 1..100u64 {
            let r = p.frame(t0 + ms(i * 9), if i == 50 { ms(200) } else { ms(3) });
            assert!(r.is_none(), "frame {i} within the second");
        }
        let (frames, max, last) = p.frame(t0 + ms(1000), ms(4)).expect("due after 1 s");
        assert_eq!((frames, max, last), (101, ms(200), ms(4)));
        assert!(p.frame(t0 + ms(1500), ms(1)).is_none());
        let (_, max, _) = p.frame(t0 + ms(2000), ms(2)).unwrap();
        assert_eq!(max, ms(2), "max restarts each window");
    }

    #[test]
    fn write_is_atomic_private_and_replaces() {
        let d = tmpdir("atomic");
        let dir = d.join("apple-kb-monitor");
        std::fs::create_dir_all(&d).unwrap();
        ensure_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        write_atomic(&dir, "{\"a\":1}").unwrap();
        write_atomic(&dir, "{\"a\":2}").unwrap();
        let f = dir.join(FILE_NAME);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{\"a\":2}\n");
        assert_eq!(std::fs::metadata(&f).unwrap().mode() & 0o777, 0o600);
        assert!(!dir.join(TMP_NAME).exists(), "no temporary file left");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn existing_loose_dir_is_tightened_and_symlink_refused() {
        let d = tmpdir("dir");
        std::fs::create_dir_all(&d).unwrap();
        let loose = d.join("loose");
        std::fs::DirBuilder::new().mode(0o755).create(&loose).unwrap();
        ensure_dir(&loose).unwrap();
        assert_eq!(std::fs::metadata(&loose).unwrap().mode() & 0o777, 0o700);
        let link = d.join("link");
        std::os::unix::fs::symlink(&loose, &link).unwrap();
        assert!(ensure_dir(&link).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn beats_only_when_the_ui_ticks_and_is_removed_on_close() {
        let d = tmpdir("life");
        let dir = d.join("apple-kb-monitor");
        std::fs::create_dir_all(&d).unwrap();
        let mut hb = Heartbeat::start(dir.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!dir.join(FILE_NAME).exists(), "the writer thread never pulses by itself");
        hb.tick(Duration::from_millis(7));
        let v: serde_json::Value = serde_json::from_str(&wait_for(&dir.join(FILE_NAME))).unwrap();
        assert_eq!(v["pid"], std::process::id());
        assert_eq!(v["frames"], 1);
        drop(hb);
        assert!(!dir.join(FILE_NAME).exists(), "removed on clean close");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tick_never_blocks_when_the_writer_is_stuck() {
        let d = tmpdir("full");
        let dir = d.join("apple-kb-monitor");
        std::fs::create_dir_all(&d).unwrap();
        let mut hb = Heartbeat::start(dir).unwrap();
        // Fill the channel, then keep sending: try_send must drop, not wait.
        let t = Instant::now();
        for _ in 0..1000 {
            if let Some(tx) = &hb.tx {
                let _ = tx.try_send(String::new());
            }
            hb.pacer.last_write = None; // force a line on every tick
            hb.tick(Duration::from_millis(1));
        }
        assert!(t.elapsed() < Duration::from_secs(2));
        drop(hb);
        let _ = std::fs::remove_dir_all(&d);
    }
}
