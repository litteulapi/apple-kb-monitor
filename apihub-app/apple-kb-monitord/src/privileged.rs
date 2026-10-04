//! The daemon's one way to run a privileged helper through pkexec (Fn mode, key mapping): one
//! single-flight guard with a cool-down for the whole process, and bounded child processes.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::settings::COOLDOWN;

/// Longest wait for pkexec (authentication dialog included).
pub const HELPER_TIMEOUT: Duration = Duration::from_mins(2);

/// Why a privileged run cannot start now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    /// Another authentication is pending.
    Pending,
    /// The previous one ended less than [`COOLDOWN`] ago; the wait left.
    CoolDown(Duration),
}

#[derive(Debug, Default)]
struct Guard {
    running: bool,
    last_end: Option<Instant>,
}

/// Single flight + cool-down; clones share the same state.
#[derive(Debug, Default, Clone)]
pub struct Flight(Arc<Mutex<Guard>>);

impl Flight {
    /// The guard shared by every privileged caller of this process: one polkit dialog at a time.
    #[must_use]
    pub fn shared() -> Self {
        static SHARED: OnceLock<Flight> = OnceLock::new();
        SHARED.get_or_init(Self::default).clone()
    }

    /// # Errors
    /// [`Busy`] while another run is pending or cooling down.
    pub fn acquire(&self) -> Result<(), Busy> {
        let mut g = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.running {
            return Err(Busy::Pending);
        }
        if let Some(left) = g
            .last_end
            .and_then(|t| COOLDOWN.checked_sub(t.elapsed()))
            .filter(|d| !d.is_zero())
        {
            return Err(Busy::CoolDown(left));
        }
        g.running = true;
        Ok(())
    }

    pub fn release(&self) {
        let mut g = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.running = false;
        g.last_end = Some(Instant::now());
    }
}

/// `pkexec <helper> <args>`, bounded by [`HELPER_TIMEOUT`].
///
/// # Errors
/// Why pkexec could not run, the helper's failure with its stderr, or the timeout.
pub fn run(pkexec: &Path, helper: &Path, args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new(pkexec);
    cmd.arg(helper).args(args);
    let f =
        run_bounded(&mut cmd, HELPER_TIMEOUT).map_err(|e| tr!("cannot run pkexec: {e}", e = e))?;
    match f.status {
        Some(st) if st.success() => Ok(()),
        Some(st) => Err(tr!(
            "helper failed ({st}): {err}",
            st = st,
            err = f.err.trim()
        )),
        None => Err(tr!("helper timed out")),
    }
}

/// What a bounded child left.
#[derive(Debug)]
pub struct Finished {
    /// `None`: still running at the limit (killed, or reaped in the background).
    pub status: Option<ExitStatus>,
    pub out: String,
    pub err: String,
}

/// Run `cmd` (stdin null, stdout and stderr captured) for at most `limit`. Both pipes are drained
/// while it runs, so a large output cannot block it. A child that cannot be killed (pkexec after
/// authentication runs as root) is reaped by a detached thread.
///
/// # Errors
/// The spawn error.
pub fn run_bounded(cmd: &mut Command, limit: Duration) -> std::io::Result<Finished> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) | Err(_) => break None,
        }
    };
    if status.is_none() && !kill_and_reap(&mut child) {
        let _ = std::thread::Builder::new().spawn(move || child.wait());
        return Ok(Finished {
            status,
            out: String::new(),
            err: String::new(),
        });
    }
    Ok(Finished {
        status,
        out: joined(out),
        err: joined(err),
    })
}

fn kill_and_reap(child: &mut Child) -> bool {
    child.kill().is_ok() && child.wait().is_ok()
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Option<JoinHandle<Vec<u8>>> {
    let mut pipe = pipe?;
    std::thread::Builder::new()
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
        .ok()
}

fn joined(h: Option<JoinHandle<Vec<u8>>>) -> String {
    h.and_then(|h| h.join().ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_beyond_the_pipe_capacity_does_not_block() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args([
            "-c",
            "head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2",
        ]);
        let f = run_bounded(&mut cmd, Duration::from_secs(20)).unwrap();
        assert!(f.status.is_some_and(|s| s.success()), "timed out");
        assert_eq!((f.out.len(), f.err.len()), (300_000, 300_000));
    }

    #[test]
    fn a_child_past_the_limit_is_killed() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo started; exec sleep 30"]);
        let t = Instant::now();
        let f = run_bounded(&mut cmd, Duration::from_millis(300)).unwrap();
        assert!(f.status.is_none() && t.elapsed() < Duration::from_secs(10));
        assert_eq!(f.out, "started\n");
    }

    #[test]
    fn one_flight_for_all_callers() {
        let (a, b) = (Flight::shared(), Flight::shared());
        let own = Flight::default();
        own.acquire().unwrap();
        assert_eq!(own.clone().acquire(), Err(Busy::Pending));
        own.release();
        assert!(matches!(own.acquire(), Err(Busy::CoolDown(_))));
        assert!(
            Arc::ptr_eq(&a.0, &b.0),
            "keymap and Fn mode share one guard"
        );
    }
}
