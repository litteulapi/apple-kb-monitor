//! `akm-helper` — the only privileged piece of apple-kb-monitor.
//!
//! Installed at `/usr/lib/apple-kb-monitor/akm-helper` and started through
//! `pkexec` (polkit action `com.agenceapi.AppleKbMonitor.set-fnmode`).
//!
//! ```text
//! akm-helper set-fnmode <0|1|2|3> [--persist]
//! ```
//!
//! * writes `/sys/module/hid_apple/parameters/fnmode` (root-only, mode 644);
//! * `--persist` first rewrites the `fnmode=` token of `/etc/modprobe.d/hid_apple.conf`
//!   (other options and comments kept; file forced to 0644 root:root, symlinks
//!   refused, atomic rename) and then tolerates an unloaded module.
//!
//! Security stance: the argument list is a strict whitelist (one verb, one
//! value in 0..=3, one optional flag); no path, no environment variable and
//! no stdin is ever used to pick a file or a value.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

const SYSFS_FNMODE: &str = "/sys/module/hid_apple/parameters/fnmode";
const MODPROBE_CONF: &str = "/etc/modprobe.d/hid_apple.conf";

/// Exit codes: 0 OK, 1 write error, 64 invalid usage (sysexits EX_USAGE).
const EX_USAGE: u8 = 64;

#[derive(Debug, PartialEq, Eq)]
struct Request {
    mode: u8,
    persist: bool,
}

/// Strict parser: exactly `set-fnmode <0..=3> [--persist]`.
fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Request, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let (verb, val, flag) = match a.as_slice() {
        [v, n] => (*v, *n, None),
        [v, n, f] => (*v, *n, Some(*f)),
        _ => return Err("usage: akm-helper set-fnmode <0-3> [--persist]".into()),
    };
    if verb != "set-fnmode" {
        return Err(format!("unknown command {verb:?}"));
    }
    let persist = match flag {
        None => false,
        Some("--persist") => true,
        Some(f) => return Err(format!("unknown option {f:?}")),
    };
    // Exactly one ASCII digit: rejects "", "+1", "01", " 1", "1\n", "٢", "10"...
    let mode = match val.as_bytes() {
        [d @ b'0'..=b'3'] => d - b'0',
        _ => return Err(format!("invalid fnmode {val:?} (expected 0, 1, 2 or 3)")),
    };
    Ok(Request { mode, persist })
}

/// New content of the modprobe file: replaces the `fnmode=` token of every
/// `options hid_apple ...` line, or appends one line. Other lines untouched.
fn merge_fnmode(existing: &str, mode: u8) -> String {
    let mut out = String::new();
    let mut done = false;
    for line in existing.lines() {
        let mut toks = line.split_whitespace();
        if toks.next() == Some("options") && toks.next() == Some("hid_apple") {
            let mut replaced = false;
            let mut parts: Vec<String> = Vec::new();
            for t in line.split_whitespace() {
                if t.starts_with("fnmode=") {
                    if !replaced {
                        parts.push(format!("fnmode={mode}"));
                        replaced = true;
                    }
                } else {
                    parts.push(t.to_string());
                }
            }
            if !replaced {
                parts.push(format!("fnmode={mode}"));
            }
            out.push_str(&parts.join(" "));
            done = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !done {
        out.push_str(&format!("options hid_apple fnmode={mode}\n"));
    }
    out
}

/// `O_NOFOLLOW` for the temp file and the read of the existing file.
const O_NOFOLLOW: i32 = libc::O_NOFOLLOW;

extern "C" {
    fn umask(mask: u32) -> u32;
}

/// Called first thing in `main`: whatever umask `pkexec` inherited from the
/// caller (an unprivileged process may pick 000), root-created files never
/// start world-writable.
fn lock_umask() {
    // SAFETY: umask(2) only changes the process file-mode creation mask.
    unsafe { umask(0o077) };
}

/// Reads the existing config without following a symlink and refuses
/// anything that is not a regular file.
fn read_existing(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = match OpenOptions::new().read(true).custom_flags(O_NOFOLLOW).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(std::io::Error::new(e.kind(), format!("{} (symlink refused?): {e}", path.display()))),
    };
    if !f.metadata()?.file_type().is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// Atomic rewrite: temp file created `O_EXCL|O_NOFOLLOW` with mode 0600,
/// forced to 0644 root:root (explicit chmod/chown, independent of umask),
/// fsync, then `rename`. Refuses a symlink at the destination.
fn persist(path: &Path, mode: u8) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Ok(md) = fs::symlink_metadata(path) {
        if !md.file_type().is_file() {
            return Err(std::io::Error::other(format!("{} is not a regular file (symlink?)", path.display())));
        }
    }
    let new = merge_fnmode(&read_existing(path)?, mode);
    let tmp = path.with_extension("conf.akm-tmp");
    let open_tmp = || {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(O_NOFOLLOW)
            .open(&tmp)
    };
    let mut f = open_tmp().or_else(|_| {
        // stale temp from a crashed run (remove_file never follows a link)
        let _ = fs::remove_file(&tmp);
        open_tmp()
    })?;
    let res = (|| {
        f.set_permissions(fs::Permissions::from_mode(0o644))?;
        // SAFETY: plain fchown(2) on an fd we own.
        let rc = unsafe { libc::fchown(f.as_raw_fd(), 0, 0) };
        // As root this must succeed; unprivileged (unit tests) it cannot.
        if rc != 0 && unsafe { libc::geteuid() } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        f.write_all(new.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

fn set_sysfs(path: &Path, mode: u8) -> std::io::Result<()> {
    // No create: the parameter must exist (hid_apple loaded).
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.write_all(format!("{mode}\n").as_bytes())
}

/// Outcome of a request, for the exit code and the message.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Applied,
    /// Persisted; `hid_apple` not loaded, applied at next module load.
    PersistedOnly,
    Failed(String),
}

/// With `--persist` the config is written FIRST and independently of sysfs
/// (module not loaded = parameter file absent is not an error then).
fn run(req: &Request, sysfs: &Path, conf: &Path) -> Outcome {
    if req.persist {
        if let Err(e) = persist(conf, req.mode) {
            return Outcome::Failed(format!("{} not updated: {e}", conf.display()));
        }
    }
    match set_sysfs(sysfs, req.mode) {
        Ok(()) => Outcome::Applied,
        Err(e) if req.persist && e.kind() == std::io::ErrorKind::NotFound => Outcome::PersistedOnly,
        Err(e) if req.persist => {
            Outcome::Failed(format!("{} updated but cannot write {}: {e}", conf.display(), sysfs.display()))
        }
        Err(e) => Outcome::Failed(format!("cannot write {}: {e}", sysfs.display())),
    }
}

fn main() -> ExitCode {
    lock_umask();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse_args(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akm-helper: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    match run(&req, Path::new(SYSFS_FNMODE), Path::new(MODPROBE_CONF)) {
        Outcome::Applied => ExitCode::SUCCESS,
        Outcome::PersistedOnly => {
            eprintln!("akm-helper: hid_apple not loaded: fnmode={} saved, applied at next module load", req.mode);
            ExitCode::SUCCESS
        }
        Outcome::Failed(m) => {
            eprintln!("akm-helper: {m}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        for n in 0..=3u8 {
            assert_eq!(
                parse_args(&["set-fnmode", &n.to_string()]),
                Ok(Request { mode: n, persist: false })
            );
        }
        assert_eq!(
            parse_args(&["set-fnmode", "2", "--persist"]),
            Ok(Request { mode: 2, persist: true })
        );
    }

    #[test]
    fn rejects_invalid_values() {
        for v in ["4", "9", "10", "-1", "+1", "01", "", " 1", "1\n", "a", "1;", "١", "0x1", "../x"] {
            assert!(parse_args(&["set-fnmode", v]).is_err(), "{v:?}");
        }
    }

    #[test]
    fn rejects_bad_shape() {
        let none: [&str; 0] = [];
        assert!(parse_args(&none).is_err());
        assert!(parse_args(&["set-fnmode"]).is_err());
        assert!(parse_args(&["set-isolayout", "1"]).is_err());
        assert!(parse_args(&["set-fnmode", "1", "--force"]).is_err());
        assert!(parse_args(&["set-fnmode", "1", "--persist", "x"]).is_err());
        assert!(parse_args(&["/etc/passwd", "1"]).is_err());
    }

    #[test]
    fn merge_creates_and_replaces() {
        assert_eq!(merge_fnmode("", 2), "options hid_apple fnmode=2\n");
        assert_eq!(merge_fnmode("options hid_apple fnmode=1\n", 3), "options hid_apple fnmode=3\n");
        assert_eq!(
            merge_fnmode("# c\noptions hid_apple iso_layout=0 fnmode=1 swap_opt_cmd=1\noptions foo a=1\n", 2),
            "# c\noptions hid_apple iso_layout=0 fnmode=2 swap_opt_cmd=1\noptions foo a=1\n"
        );
        assert_eq!(
            merge_fnmode("options hid_apple iso_layout=1\n", 0),
            "options hid_apple iso_layout=1 fnmode=0\n"
        );
        assert_eq!(merge_fnmode("# only comment\n", 1), "# only comment\noptions hid_apple fnmode=1\n");
    }

    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::Mutex;

    static UMASK: Mutex<()> = Mutex::new(());

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("akm-helper-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn perm(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    /// The pre-fix implementation, kept to reproduce the defect (#152).
    fn persist_legacy(path: &Path, mode: u8) -> std::io::Result<()> {
        let new = merge_fnmode("", mode);
        let tmp = path.with_extension("conf.akm-tmp");
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(new.as_bytes())?;
        fs::rename(&tmp, path)
    }

    #[test]
    fn umask_000_old_defect_then_fix() {
        let _g = UMASK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tmpdir("umask");
        // SAFETY: test-only, serialised by UMASK.
        let old = unsafe { umask(0) };
        let legacy = d.join("legacy.conf");
        persist_legacy(&legacy, 1).unwrap();
        let fixed = d.join("hid_apple.conf");
        persist(&fixed, 1).unwrap();
        unsafe { umask(0o077) };
        let strict = d.join("strict.conf");
        persist(&strict, 1).unwrap();
        unsafe { umask(old) };
        assert_eq!(perm(&legacy), 0o666, "old code: world-writable under umask 000");
        assert_eq!(perm(&fixed), 0o644, "umask 000 must still give 0644");
        assert_eq!(perm(&strict), 0o644, "umask 077 must still give 0644");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn existing_world_writable_file_is_tightened() {
        let d = tmpdir("tighten");
        let p = d.join("hid_apple.conf");
        fs::write(&p, "options hid_apple fnmode=1\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
        persist(&p, 2).unwrap();
        assert_eq!(perm(&p), 0o644);
        assert!(!d.join("hid_apple.conf.akm-tmp").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn refuses_symlink_destination() {
        let d = tmpdir("symlink");
        let target = d.join("victim");
        fs::write(&target, "keep\n").unwrap();
        let p = d.join("hid_apple.conf");
        symlink(&target, &p).unwrap();
        assert!(persist(&p, 2).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep\n");
        assert!(fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn stale_symlink_tmp_is_not_followed() {
        let d = tmpdir("staletmp");
        let target = d.join("victim");
        fs::write(&target, "keep\n").unwrap();
        symlink(&target, d.join("hid_apple.conf.akm-tmp")).unwrap();
        let p = d.join("hid_apple.conf");
        persist(&p, 3).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep\n");
        assert_eq!(fs::read_to_string(&p).unwrap(), "options hid_apple fnmode=3\n");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn persist_roundtrip_in_tempdir() {
        let d = tmpdir("rt");
        let p = d.join("hid_apple.conf");
        persist(&p, 2).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "options hid_apple fnmode=2\n");
        persist(&p, 0).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "options hid_apple fnmode=0\n");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn persist_without_module_loaded_still_persists() {
        let d = tmpdir("nomod");
        let conf = d.join("hid_apple.conf");
        let sysfs = d.join("absent/fnmode");
        let req = Request { mode: 2, persist: true };
        assert_eq!(run(&req, &sysfs, &conf), Outcome::PersistedOnly);
        assert_eq!(fs::read_to_string(&conf).unwrap(), "options hid_apple fnmode=2\n");
        // without --persist a missing module stays an error and writes nothing
        let conf2 = d.join("other.conf");
        let req = Request { mode: 2, persist: false };
        assert!(matches!(run(&req, &sysfs, &conf2), Outcome::Failed(_)));
        assert!(!conf2.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn persist_with_module_loaded_writes_both() {
        let d = tmpdir("both");
        let conf = d.join("hid_apple.conf");
        let sysfs = d.join("fnmode");
        fs::write(&sysfs, "1\n").unwrap();
        let req = Request { mode: 3, persist: true };
        assert_eq!(run(&req, &sysfs, &conf), Outcome::Applied);
        assert_eq!(fs::read_to_string(&sysfs).unwrap(), "3\n");
        assert_eq!(fs::read_to_string(&conf).unwrap(), "options hid_apple fnmode=3\n");
        fs::remove_dir_all(&d).unwrap();
    }
}
