//! Atomic replacement of the user's files (config, keymap, state).

use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Permissions of the file [`write_atomic`] puts in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// This mode (minus the umask) on every write: private state files.
    Fixed(u32),
    /// The replaced file's permissions; this mode (minus the umask) for a new file.
    Keep(u32),
}

/// Temp file next to `path`, unique to this process and call.
fn temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// Replaces `path` with `bytes`: unique temp file, data fsync, rename, directory fsync.
///
/// A symlinked `path` (stow, home-manager) is written through to its target; the parent is created.
///
/// # Errors
///
/// Any I/O error while creating, writing or renaming the file (the temp file is then removed).
pub fn write_atomic(path: &Path, bytes: &[u8], mode: Mode) -> io::Result<()> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(d) = dir {
        std::fs::create_dir_all(d)?;
    }
    let (new_mode, keep) = match mode {
        Mode::Fixed(m) => (m, None),
        Mode::Keep(m) => (m, std::fs::metadata(&path).ok().map(|m| m.permissions())),
    };
    let tmp = temp_path(&path);
    let written = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(new_mode)
            .open(&tmp)?;
        if let Some(p) = keep {
            f.set_permissions(Permissions::from_mode(p.mode() & 0o7777))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // The file is in place: a failed directory fsync must not report the write as failed.
    let _ = File::open(dir.unwrap_or(Path::new("."))).and_then(|d| d.sync_all());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("akm-fsutil-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode_of(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn writes_through_a_symlink_and_keeps_it() {
        let d = dir("link");
        std::fs::write(d.join("real.toml"), "old").unwrap();
        std::os::unix::fs::symlink(d.join("real.toml"), d.join("link.toml")).unwrap();
        write_atomic(&d.join("link.toml"), b"new", Mode::Keep(0o644)).unwrap();
        assert!(std::fs::symlink_metadata(d.join("link.toml"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(d.join("real.toml")).unwrap(), "new");
        assert_eq!(
            std::fs::read_dir(&d).unwrap().count(),
            2,
            "no temp file left"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn keep_preserves_the_mode_and_fixed_imposes_it() {
        let d = dir("mode");
        let (k, f) = (d.join("sub/keep"), d.join("sub/fixed"));
        write_atomic(&k, b"1", Mode::Keep(0o600)).unwrap();
        assert_eq!(mode_of(&k), 0o600);
        std::fs::set_permissions(&k, Permissions::from_mode(0o640)).unwrap();
        write_atomic(&k, b"2", Mode::Keep(0o600)).unwrap();
        assert_eq!(mode_of(&k), 0o640);
        std::fs::write(&f, "0").unwrap();
        std::fs::set_permissions(&f, Permissions::from_mode(0o644)).unwrap();
        write_atomic(&f, b"1", Mode::Fixed(0o600)).unwrap();
        assert_eq!(mode_of(&f), 0o600);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn concurrent_writers_never_share_a_temp_file() {
        let d = dir("race");
        let p = d.join("state.json");
        std::thread::scope(|s| {
            for i in 0..8u8 {
                let p = &p;
                s.spawn(move || {
                    for _ in 0..50 {
                        write_atomic(p, &[b'a' + i; 4096], Mode::Fixed(0o600)).unwrap();
                    }
                });
            }
        });
        let c = std::fs::read(&p).unwrap();
        assert!(c.len() == 4096 && c.iter().all(|b| *b == c[0]), "torn file");
        assert_eq!(
            std::fs::read_dir(&d).unwrap().count(),
            1,
            "no temp file left"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
