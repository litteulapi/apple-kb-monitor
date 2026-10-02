//! External commands of the Diag tab, bounded in time (#235): a command that
//! hangs (stuck `systemctl`, broken `keyd`) is killed after
//! [`COMMAND_TIMEOUT`] instead of leaving "Running..." for ever, and every
//! child dies with this process (`PR_SET_PDEATHSIG`) instead of surviving as
//! an orphan when the window is closed.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// Not found / not executable.
    Spawn,
    /// Still running after the timeout: killed and reaped.
    Timeout,
}

/// Read a pipe to its end on a helper thread (a grandchild may keep it open:
/// the caller waits for it only until the deadline).
fn drain(r: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// `cmd.output()` with a deadline; stdin is closed.
pub fn run_bounded(cmd: &mut Command, timeout: Duration) -> Result<Output, RunError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: prctl is async-signal-safe; nothing else runs between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|_| RunError::Spawn)?;
    let (out, err) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => break None,
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(RunError::Timeout);
    };
    let rest = |rx: mpsc::Receiver<Vec<u8>>| {
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
    };
    Ok(Output {
        status,
        stdout: rest(out),
        stderr: rest(err),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_and_status_are_kept() {
        let o = run_bounded(
            Command::new("sh").args(["-c", "echo hi; echo err >&2; exit 3"]),
            COMMAND_TIMEOUT,
        )
        .unwrap();
        assert_eq!(o.status.code(), Some(3));
        assert_eq!(o.stdout, b"hi\n");
        assert_eq!(o.stderr, b"err\n");
    }

    #[test]
    fn a_hung_command_is_killed_at_the_deadline() {
        let t = Instant::now();
        let r = run_bounded(
            Command::new("sleep").arg("100000"),
            Duration::from_millis(300),
        );
        assert_eq!(r.unwrap_err(), RunError::Timeout);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn a_grandchild_keeping_the_pipe_open_does_not_block_the_caller() {
        // The shell exits at once but leaves a background child holding stdout.
        let t = Instant::now();
        let r = run_bounded(
            Command::new("sh").args(["-c", "sleep 5 & echo ok"]),
            Duration::from_millis(400),
        );
        assert!(r.unwrap().status.success());
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn missing_binary_is_reported() {
        assert_eq!(
            run_bounded(&mut Command::new("/nonexistent/akm-diag"), COMMAND_TIMEOUT).unwrap_err(),
            RunError::Spawn
        );
    }
}
