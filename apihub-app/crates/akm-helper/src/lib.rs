//! Shared code of the two privileged programs of apple-kb-monitor.

#[path = "../../../akm-core/src/breaker_state.rs"]
pub mod breaker_state;
#[path = "../../../akm-core/src/hid_params.rs"]
pub mod hid_params;
// Shared with akm-core: gettext through glibc, no extra dependency in the privileged helper.
#[path = "../../../akm-core/src/i18n.rs"]
pub mod i18n;
#[path = "../../../akm-core/src/keycodes.rs"]
pub mod keycodes;
#[path = "../../../akm-core/src/keymap.rs"]
pub mod keymap;
#[path = "../../../akm-core/src/model.rs"]
pub mod model;
#[path = "../../../akm-core/src/paths.rs"]
pub mod paths;

pub mod doctor_apply;
pub mod doctor_fix;
pub mod hidctl;

/// sysexits(3) `EX_USAGE`: every subcommand exits with it on a bad command line.
pub const EX_USAGE: u8 = 64;

pub mod fsutil {
    //! File primitives run as root.

    use crate::tr;
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    /// Called first thing in `main`.
    pub fn lock_umask() {
        let _previous = set_umask(0o077);
    }

    /// Sets the process umask and returns the previous one.
    #[must_use]
    pub fn set_umask(m: libc::mode_t) -> libc::mode_t {
        // SAFETY: umask(2) only changes the process file-mode creation mask.
        unsafe { libc::umask(m) }
    }

    /// Read a regular file of at most `max` bytes (absent = `None`, symlink refused).
    ///
    /// # Errors
    ///
    /// Any I/O error (a symlink is refused), or when the file is not regular or larger than `max`.
    pub fn read_regular(path: &Path, max: u64) -> std::io::Result<Option<String>> {
        let mut f = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(std::io::Error::new(
                    e.kind(),
                    tr!(
                        "{path} (symlink refused?): {e}",
                        path = path.display(),
                        e = e
                    ),
                ))
            }
        };
        let md = f.metadata()?;
        if !md.file_type().is_file() {
            return Err(std::io::Error::other(tr!(
                "{} is not a regular file",
                path.display()
            )));
        }
        if md.len() > max {
            return Err(std::io::Error::other(tr!(
                "{path} is larger than {max} bytes",
                path = path.display(),
                max = max
            )));
        }
        let mut s = String::new();
        Read::take(&mut f, max + 1).read_to_string(&mut s)?;
        Ok(Some(s))
    }

    /// Like [`read_regular`] for a file written by an unprivileged user.
    ///
    /// # Errors
    ///
    /// As [`read_regular`], plus a refusal when the file is not owned by `uid`.
    pub fn read_user_file(path: &Path, uid: u32, max: u64) -> std::io::Result<String> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let md = f.metadata()?;
        if !md.file_type().is_file() {
            return Err(std::io::Error::other(tr!(
                "{} is not a regular file",
                path.display()
            )));
        }
        if md.uid() != uid {
            return Err(std::io::Error::other(tr!(
                "{} is not owned by the requesting user",
                path.display()
            )));
        }
        if md.nlink() != 1 {
            return Err(std::io::Error::other(tr!(
                "{} has several hard links",
                path.display()
            )));
        }
        if md.mode() & 0o022 != 0 {
            return Err(std::io::Error::other(tr!(
                "{} is group or world writable",
                path.display()
            )));
        }
        if md.len() > max {
            return Err(std::io::Error::other(tr!(
                "{path} is larger than {max} bytes",
                path = path.display(),
                max = max
            )));
        }
        let mut s = String::new();
        f.take(max + 1).read_to_string(&mut s)?;
        Ok(s)
    }

    /// Atomic write (temporary file + rename), no symlink followed.
    // Root writer: refuses links, unlike akm_core::fsutil::write_atomic which writes through the user's.
    ///
    /// # Errors
    ///
    /// Any I/O error while creating, writing or renaming the file.
    pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
        if let Ok(md) = fs::symlink_metadata(path) {
            if !md.file_type().is_file() {
                return Err(std::io::Error::other(tr!(
                    "{} is not a regular file (symlink?)",
                    path.display()
                )));
            }
        }
        let mut name = path
            .file_name()
            .map(std::ffi::OsStr::to_os_string)
            .unwrap_or_default();
        name.push(".akm-tmp");
        let tmp = path.with_file_name(name);
        let open_tmp = || {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
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
            f.write_all(content)?;
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

    /// Removes a regular file (absent = fine, symlink = removed itself, never followed).
    ///
    /// # Errors
    ///
    /// Any I/O error other than `NotFound`.
    pub fn remove(path: &Path) -> std::io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

pub mod params {
    //! `hid_apple` parameters: whitelist (akm-core), sysfs, modprobe.d.

    use crate::tr;
    use std::fmt::Write as _;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::Path;

    use crate::keymap::{kernel_param, parse_param_value};

    use crate::fsutil;

    #[must_use]
    pub fn merge(existing: &str, set: &[(&str, i32)]) -> String {
        let mut out = String::new();
        let mut done = false;
        for line in existing.lines() {
            let mut toks = line.split_whitespace();
            if toks.next() == Some("options") && toks.next() == Some("hid_apple") {
                let mut seen = vec![false; set.len()];
                let mut parts: Vec<String> = Vec::new();
                for t in line.split_whitespace() {
                    match set
                        .iter()
                        .position(|(n, _)| t.split_once('=').is_some_and(|(k, _)| k == *n))
                    {
                        Some(i) if !seen[i] => {
                            parts.push(format!("{}={}", set[i].0, set[i].1));
                            seen[i] = true;
                        }
                        Some(_) => {}
                        None => parts.push(t.to_string()),
                    }
                }
                for (i, (n, v)) in set.iter().enumerate() {
                    if !seen[i] {
                        parts.push(format!("{n}={v}"));
                    }
                }
                out.push_str(&parts.join(" "));
                done = true;
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        if !done {
            let opts: Vec<String> = set.iter().map(|(n, v)| format!("{n}={v}")).collect();
            let _ = writeln!(out, "options hid_apple {}", opts.join(" "));
        }
        out
    }

    /// Parse `name=value` for a whitelisted `hid_apple` parameter.
    ///
    /// # Errors
    ///
    /// A message when the name or the value is not allowed.
    pub fn parse_assignment(s: &str) -> Result<(&'static str, i32), String> {
        let (n, v) = s
            .split_once('=')
            .ok_or_else(|| tr!("expected NAME=VALUE, got {s}", s = format!("{:?}", s)))?;
        let p = kernel_param(n)
            .ok_or_else(|| tr!("parameter {n} not allowed", n = format!("{:?}", n)))?;
        let v = parse_param_value(p, v).map_err(|e| e.to_string())?;
        Ok((p.name, v))
    }

    /// Write the `hid_apple` options file for `set`.
    ///
    /// # Errors
    ///
    /// Any I/O error from [`fsutil::atomic_write`].
    pub fn persist(conf: &Path, set: &[(&str, i32)]) -> std::io::Result<()> {
        let existing = fsutil::read_regular(conf, 64 * 1024)?.unwrap_or_default();
        fsutil::atomic_write(conf, merge(&existing, set).as_bytes())
    }

    /// Write `v` to the sysfs parameter `dir/name`.
    ///
    /// # Errors
    ///
    /// Any I/O error while opening or writing the file.
    pub fn write_sysfs(dir: &Path, name: &str, v: i32) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        // No create: the parameter must exist (hid_apple loaded).
        let mut f = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join(name))?;
        f.write_all(format!("{v}\n").as_bytes())
    }
}

pub mod keymap_install {
    //! Install / remove / roll back the hwdb file of the manual key mapping.

    use crate::{tr, trn};
    use std::path::{Path, PathBuf};

    use crate::keymap::{self, parse_hwdb, render_hwdb, transition, HwdbRecord, MAX_HWDB};

    use crate::fsutil;

    pub const TARGET: &str = keymap::HWDB_PATH;
    pub const SYSTEMD_HWDB: &str = "/usr/bin/systemd-hwdb";
    pub const UDEVADM: &str = "/usr/bin/udevadm";

    #[derive(Debug, Clone)]
    pub struct Paths {
        pub source: PathBuf,
        pub target: PathBuf,
        pub backup: PathBuf,
    }

    impl Paths {
        #[must_use]
        pub fn for_uid(uid: u32) -> Paths {
            Paths::new(source_for_uid(uid), PathBuf::from(TARGET))
        }

        #[must_use]
        pub fn new(source: PathBuf, target: PathBuf) -> Paths {
            let mut b = target
                .file_name()
                .map(std::ffi::OsStr::to_os_string)
                .unwrap_or_default();
            b.push(".akm-bak"); // not *.hwdb: systemd-hwdb ignores it
            Paths {
                source,
                backup: target.with_file_name(b),
                target,
            }
        }
    }

    #[must_use]
    pub fn source_for_uid(uid: u32) -> PathBuf {
        crate::paths::keymap_source_for_uid(uid)
    }

    /// Parse a numeric user id.
    ///
    /// # Errors
    ///
    /// A message when `s` is not a decimal `u32`.
    pub fn parse_uid(s: &str) -> Result<u32, String> {
        if s.is_empty()
            || s.len() > 10
            || !s.bytes().all(|b| b.is_ascii_digit())
            || (s.len() > 1 && s.starts_with('0'))
        {
            return Err(tr!("bad PKEXEC_UID {s}", s = format!("{:?}", s)));
        }
        s.parse()
            .map_err(|_| tr!("bad PKEXEC_UID {s}", s = format!("{:?}", s)))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Verb {
        Install,
        Remove,
        Rollback,
    }

    /// Parse the command line of the helper.
    ///
    /// # Errors
    ///
    /// A message naming the invalid argument.
    pub fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Verb, String> {
        match args.iter().map(AsRef::as_ref).collect::<Vec<_>>().as_slice() {
            ["install"] => Ok(Verb::Install),
            ["remove"] => Ok(Verb::Remove),
            ["rollback"] => Ok(Verb::Rollback),
            _ => Err(tr!("usage: akm-helper install-keymap install|remove|rollback (the file is read from /run/user/$PKEXEC_UID/apple-kb-monitor/keymap.hwdb)")),
        }
    }

    pub trait Runner {
        /// Run `argv`.
        ///
        /// # Errors
        ///
        /// A message when the program cannot start or fails.
        fn run(&self, argv: &[&str]) -> Result<(), String>;
    }

    pub struct SystemRunner;

    impl Runner for SystemRunner {
        fn run(&self, argv: &[&str]) -> Result<(), String> {
            let st = std::process::Command::new(argv[0])
                .args(&argv[1..])
                .env_clear()
                .stdin(std::process::Stdio::null())
                .status()
                .map_err(|e| format!("{}: {e}", argv[0]))?;
            if st.success() {
                Ok(())
            } else {
                Err(tr!(
                    "{value} failed ({st})",
                    value = argv.join(" "),
                    st = st
                ))
            }
        }
    }

    fn reload(r: &dyn Runner) -> Result<(), String> {
        r.run(&[SYSTEMD_HWDB, "update"])?;
        r.run(&[
            UDEVADM,
            "trigger",
            "--settle",
            "--subsystem-match=input",
            "--action=change",
        ])
    }

    fn installed(path: &Path) -> Result<(Option<String>, Option<Vec<HwdbRecord>>), String> {
        Ok(
            match fsutil::read_regular(path, MAX_HWDB as u64).map_err(|e| e.to_string())? {
                None => (None, None),
                Some(s) => (Some(s.clone()), parse_hwdb(&s).ok().map(|(_, r)| r)),
            },
        )
    }

    /// Execute `verb` and return its report.
    ///
    /// # Errors
    ///
    /// A message saying which step failed.
    pub fn run(verb: Verb, p: &Paths, uid: u32, r: &dyn Runner) -> Result<String, String> {
        match verb {
            Verb::Install => install(p, uid, r),
            Verb::Remove => remove(p, r),
            Verb::Rollback => rollback(p, r),
        }
    }

    fn install(p: &Paths, uid: u32, r: &dyn Runner) -> Result<String, String> {
        let src =
            fsutil::read_user_file(&p.source, uid, MAX_HWDB as u64).map_err(|e| e.to_string())?;
        let (profile, new) = parse_hwdb(&src)
            .map_err(|e| tr!("{source} refused: {e}", source = p.source.display(), e = e))?;
        if new.is_empty() {
            return Err(tr!("nothing to install (no KEYBOARD_KEY_ line); use `remove` to go back to the kernel mapping"));
        }
        let (old_text, old) = installed(&p.target)?;
        let content = render_hwdb(
            profile.as_deref().unwrap_or("unknown"),
            &transition(&old.unwrap_or_default(), &new),
        );
        if let Some(t) = old_text {
            fsutil::atomic_write(&p.backup, t.as_bytes()).map_err(|e| e.to_string())?;
        }
        fsutil::atomic_write(&p.target, content.as_bytes()).map_err(|e| e.to_string())?;
        reload(r)?;
        Ok(trn!(
            "{} installed ({} model); previous file kept as {}",
            "{} installed ({} models); previous file kept as {}",
            new.len(),
            p.target.display(),
            new.len(),
            p.backup.display()
        ))
    }

    fn remove(p: &Paths, r: &dyn Runner) -> Result<String, String> {
        let (old_text, old) = installed(&p.target)?;
        let Some(text) = old_text else {
            return Ok(tr!(
                "nothing installed: the kernel mapping is already in use"
            ));
        };
        fsutil::atomic_write(&p.backup, text.as_bytes()).map_err(|e| e.to_string())?;
        let restore = transition(&old.unwrap_or_default(), &[]);
        if !restore.is_empty() {
            fsutil::atomic_write(
                &p.target,
                render_hwdb("restore-defaults", &restore).as_bytes(),
            )
            .map_err(|e| e.to_string())?;
            reload(r)?;
        }
        fsutil::remove(&p.target).map_err(|e| e.to_string())?;
        r.run(&[SYSTEMD_HWDB, "update"])?;
        Ok(tr!(
            "{} removed (kept as {}); kernel mapping restored",
            p.target.display(),
            p.backup.display()
        ))
    }

    fn rollback(p: &Paths, r: &dyn Runner) -> Result<String, String> {
        let bak = fsutil::read_regular(&p.backup, MAX_HWDB as u64)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| tr!("no previous mapping saved"))?;
        let (profile, prev) = parse_hwdb(&bak)
            .map_err(|e| tr!("{backup} refused: {e}", backup = p.backup.display(), e = e))?;
        let (cur_text, cur) = installed(&p.target)?;
        let content = render_hwdb(
            profile.as_deref().unwrap_or("unknown"),
            &transition(&cur.unwrap_or_default(), &prev),
        );
        match cur_text {
            Some(t) => fsutil::atomic_write(&p.backup, t.as_bytes()).map_err(|e| e.to_string())?,
            None => fsutil::remove(&p.backup).map_err(|e| e.to_string())?,
        }
        fsutil::atomic_write(&p.target, content.as_bytes()).map_err(|e| e.to_string())?;
        reload(r)?;
        Ok(tr!("previous mapping restored in {}", p.target.display()))
    }
}

#[cfg(test)]
mod install_tests {
    use std::cell::RefCell;
    use std::fs;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    use crate::keymap::{render_hwdb, HwdbRecord, HWDB_HEADER, SC_EJECT};
    use crate::keymap_install::*;

    #[derive(Default)]
    struct Rec(RefCell<Vec<String>>);
    impl Runner for Rec {
        fn run(&self, argv: &[&str]) -> Result<(), String> {
            self.0.borrow_mut().push(argv.join(" "));
            Ok(())
        }
    }

    fn uid() -> u32 {
        // SAFETY: getuid(2) has no failure mode.
        unsafe { libc::getuid() }
    }

    fn setup(tag: &str) -> (PathBuf, Paths) {
        let d = std::env::temp_dir().join(format!("akm-kmh-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("run")).unwrap();
        fs::create_dir_all(d.join("etc/udev/hwdb.d")).unwrap();
        let p = Paths::new(
            d.join("run/keymap.hwdb"),
            d.join("etc/udev/hwdb.d/90-apple-kb-monitor.hwdb"),
        );
        (d, p)
    }

    fn write_src(p: &Paths, s: &str) {
        let _ = fs::remove_file(&p.source);
        fs::write(&p.source, s).unwrap();
        fs::set_permissions(&p.source, fs::Permissions::from_mode(0o644)).unwrap();
    }

    fn rec(keys: &[(u32, u16)]) -> Vec<HwdbRecord> {
        vec![HwdbRecord {
            pid: 0x0256,
            keys: keys.iter().copied().collect(),
        }]
    }

    #[test]
    fn install_rollback_remove_cycle() {
        let (d, p) = setup("cycle");
        let r = Rec::default();
        write_src(&p, &render_hwdb("a", &rec(&[(0x7003f, 183)])));
        run(Verb::Install, &p, uid(), &r).unwrap();
        let t1 = fs::read_to_string(&p.target).unwrap();
        assert!(t1.contains(" KEYBOARD_KEY_7003f=f13\n"), "{t1}");
        assert_eq!(fs::metadata(&p.target).unwrap().mode() & 0o7777, 0o644);
        assert!(!p.backup.exists());
        assert_eq!(
            *r.0.borrow(),
            [
                "/usr/bin/systemd-hwdb update",
                "/usr/bin/udevadm trigger --settle --subsystem-match=input --action=change"
            ]
        );
        write_src(&p, &render_hwdb("b", &rec(&[(SC_EJECT, 111)])));
        run(Verb::Install, &p, uid(), &r).unwrap();
        let t2 = fs::read_to_string(&p.target).unwrap();
        assert!(
            t2.contains(" KEYBOARD_KEY_7003f=f6\n") && t2.contains(" KEYBOARD_KEY_c00b8=delete\n"),
            "{t2}"
        );
        assert_eq!(fs::read_to_string(&p.backup).unwrap(), t1);
        run(Verb::Rollback, &p, uid(), &r).unwrap();
        let t3 = fs::read_to_string(&p.target).unwrap();
        assert!(
            t3.contains("=f13\n") && t3.contains(" KEYBOARD_KEY_c00b8=ejectcd\n"),
            "{t3}"
        );
        assert_eq!(fs::read_to_string(&p.backup).unwrap(), t2);
        r.0.borrow_mut().clear();
        let m = run(Verb::Remove, &p, uid(), &r).unwrap();
        assert!(m.contains("removed"));
        assert!(!p.target.exists());
        assert_eq!(fs::read_to_string(&p.backup).unwrap(), t3);
        assert_eq!(
            r.0.borrow().len(),
            3,
            "update + trigger with defaults, then update"
        );
        assert!(run(Verb::Remove, &p, uid(), &r)
            .unwrap()
            .contains("nothing installed"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn refuses_bad_sources_and_writes_nothing() {
        let (d, p) = setup("bad");
        let r = Rec::default();
        let cases = [
            format!("{HWDB_HEADER}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\nRUN+=\"/bin/sh -c id\"\n"),
            format!("{HWDB_HEADER}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\n IMPORT{{program}}=\"x\"\n"),
            format!("{HWDB_HEADER}\nevdev:*\n KEYBOARD_KEY_7003f=f6\n"),
            format!("{HWDB_HEADER}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_7003f=f6\\\n KEYBOARD_KEY_1=a\n"),
            format!("{HWDB_HEADER}\n"),
            "not ours\n".to_string(),
        ];
        for c in &cases {
            write_src(&p, c);
            assert!(run(Verb::Install, &p, uid(), &r).is_err(), "{c:?}");
        }
        assert!(!p.target.exists() && r.0.borrow().is_empty());
        write_src(&p, &render_hwdb("a", &rec(&[(0x7003f, 183)])));
        assert!(run(Verb::Install, &p, uid() + 1, &r)
            .unwrap_err()
            .contains("not owned"));
        fs::set_permissions(&p.source, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r)
            .unwrap_err()
            .contains("writable"));
        fs::set_permissions(&p.source, fs::Permissions::from_mode(0o644)).unwrap();
        fs::hard_link(&p.source, d.join("run/link")).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r)
            .unwrap_err()
            .contains("hard links"));
        fs::remove_file(d.join("run/link")).unwrap();
        let real = d.join("run/real.hwdb");
        fs::rename(&p.source, &real).unwrap();
        symlink(&real, &p.source).unwrap();
        assert!(
            run(Verb::Install, &p, uid(), &r).is_err(),
            "symlinked source"
        );
        assert!(!p.target.exists() && r.0.borrow().is_empty());
        fs::remove_file(&p.source).unwrap();
        fs::rename(&real, &p.source).unwrap();
        let victim = d.join("victim");
        fs::write(&victim, "keep\n").unwrap();
        symlink(&victim, &p.target).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r).is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep\n");
        fs::remove_file(&p.target).unwrap();
        assert!(run(Verb::Rollback, &p, uid(), &r)
            .unwrap_err()
            .contains("no previous"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn args_and_uid_are_strict() {
        assert_eq!(parse_args(&["install"]), Ok(Verb::Install));
        assert_eq!(parse_args(&["remove"]), Ok(Verb::Remove));
        assert_eq!(parse_args(&["rollback"]), Ok(Verb::Rollback));
        for bad in [
            &[][..],
            &["install", "/tmp/x"],
            &["install", "--force"],
            &["--help"],
            &["../install"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_uid("1000"), Ok(1000));
        assert_eq!(parse_uid("0"), Ok(0));
        assert_eq!(parse_uid("7"), Ok(7));
        assert_eq!(parse_uid("4294967295"), Ok(u32::MAX));
        for bad in [
            "",
            "01000",
            "00",
            "-1",
            "1000 ",
            "abc",
            "99999999999",
            "4294967296",
            "04294967295",
        ] {
            assert!(parse_uid(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            source_for_uid(1000),
            PathBuf::from("/run/user/1000/apple-kb-monitor/keymap.hwdb")
        );
        assert_eq!(
            Paths::for_uid(1).backup,
            PathBuf::from("/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb.akm-bak")
        );
    }
}

#[cfg(test)]
mod root_writer_tests {
    #[test]
    fn no_symlink_following_writer_in_the_helper() {
        // keymap.rs is compiled as root here: its file I/O lives in akm-core's keymap_file.rs.
        let shared = include_str!("../../../akm-core/src/keymap.rs");
        assert!(!shared.contains("fsutil"));
        assert!(!include_str!("lib.rs").contains(concat!("akm-core/src/", "fsutil.rs")));
    }
}
