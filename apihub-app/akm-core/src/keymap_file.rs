//! Keymap file I/O, out of keymap.rs: the root helper includes keymap.rs and must not compile a
//! writer that follows symlinks.

use std::path::Path;

use crate::keymap::{Keymap, KeymapError};

impl Keymap {
    /// Read and parse the file at `path` (absent = default keymap).
    ///
    /// # Errors
    ///
    /// [`KeymapError`] for an I/O error or invalid content.
    pub fn load(path: &Path) -> Result<Keymap, KeymapError> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                Keymap::parse(&s).map_err(|e| KeymapError::new(format!("{}: {e}", path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Keymap::default()),
            Err(e) => Err(KeymapError::new(format!("{}: {e}", path.display()))),
        }
    }

    /// Atomic write through a symlink, mode kept (0644 when new), parent created.
    ///
    /// # Errors
    ///
    /// [`KeymapError`] wrapping the I/O error.
    pub fn save(&self, path: &Path) -> Result<(), KeymapError> {
        crate::fsutil::write_atomic(
            path,
            self.to_toml().as_bytes(),
            crate::fsutil::Mode::Keep(0o644),
        )
        .map_err(|e| KeymapError::new(format!("{}: {e}", path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str =
        "schema = 1\nactive = \"work\"\n\n[profile.default]\npreset = \"apple\"\n\n\
[profile.work]\npreset = \"linux-pc\"\n\n[profile.work.keys]\nF6 = \"KEY_F6\"\n";

    #[test]
    fn save_and_load() {
        let d = std::env::temp_dir().join(format!("akm-keymap-io-{}", std::process::id()));
        let p = d.join("sub/keymap.toml");
        assert_eq!(Keymap::load(&p).unwrap(), Keymap::default());
        let k = Keymap::parse(SAMPLE).unwrap();
        k.save(&p).unwrap();
        assert_eq!(Keymap::load(&p).unwrap(), k);
        std::fs::write(&p, "schema = 3\n").unwrap();
        assert!(Keymap::load(&p).unwrap_err().0.contains("keymap.toml"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_writes_through_a_symlinked_keymap() {
        let d = std::env::temp_dir().join(format!("akm-keymap-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (real, link) = (d.join("dotfiles.toml"), d.join("keymap.toml"));
        std::fs::write(&real, "").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let k = Keymap::parse(SAMPLE).unwrap();
        k.save(&link).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(Keymap::load(&real).unwrap(), k);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
