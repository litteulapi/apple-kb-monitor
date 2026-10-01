//! Shared code of the two privileged programs of apple-kb-monitor:
//!
//! * `akm-helper` (polkit `com.agenceapi.AppleKbMonitor.set-fnmode`):
//!   `hid_apple` parameters, sysfs + `/etc/modprobe.d/hid_apple.conf`;
//! * `akm-keymap-helper` (polkit `com.agenceapi.AppleKbMonitor.install-keymap`):
//!   the udev hwdb file of the manual key mapping (#247).
//!
//! Two programs because pkexec picks the polkit action from the program path
//! (`org.freedesktop.policykit.exec.path`): one path, one action.
//!
//! The whitelists are the very source files of `akm-core` (`keymap.rs`,
//! `keycodes.rs`, `hid_params.rs`, std only), compiled in here by path: one
//! definition, and the programs that run as root still depend on `libc` only.

#[path = "../../../akm-core/src/hid_params.rs"]
pub mod hid_params;
#[path = "../../../akm-core/src/keycodes.rs"]
pub mod keycodes;
#[path = "../../../akm-core/src/keymap.rs"]
pub mod keymap;

pub mod fsutil {
    //! File primitives run as root: never follow a symlink, never inherit a
    //! permissive mode, always replace atomically.
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    extern "C" {
        fn umask(mask: u32) -> u32;
    }

    /// Called first thing in `main`: whatever umask `pkexec` inherited from
    /// the caller, root-created files never start world-writable.
    pub fn lock_umask() {
        // SAFETY: umask(2) only changes the process file-mode creation mask.
        unsafe { umask(0o077) };
    }

    /// Test helper: set the umask, return the previous one.
    pub fn set_umask(m: u32) -> u32 {
        // SAFETY: as above.
        unsafe { umask(m) }
    }

    /// Reads a regular file without following a symlink; absent = `None`.
    pub fn read_regular(path: &Path, max: u64) -> std::io::Result<Option<String>> {
        let mut f = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(std::io::Error::new(e.kind(), format!("{} (symlink refused?): {e}", path.display()))),
        };
        let md = f.metadata()?;
        if !md.file_type().is_file() {
            return Err(std::io::Error::other(format!("{} is not a regular file", path.display())));
        }
        if md.len() > max {
            return Err(std::io::Error::other(format!("{} is larger than {max} bytes", path.display())));
        }
        let mut s = String::new();
        Read::take(&mut f, max + 1).read_to_string(&mut s)?;
        Ok(Some(s))
    }

    /// Like [`read_regular`] for a file written by an unprivileged user:
    /// must be owned by `uid`, have a single link and not be group/world
    /// writable.
    pub fn read_user_file(path: &Path, uid: u32, max: u64) -> std::io::Result<String> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let md = f.metadata()?;
        if !md.file_type().is_file() {
            return Err(std::io::Error::other(format!("{} is not a regular file", path.display())));
        }
        if md.uid() != uid {
            return Err(std::io::Error::other(format!("{} is not owned by the requesting user", path.display())));
        }
        if md.nlink() != 1 {
            return Err(std::io::Error::other(format!("{} has several hard links", path.display())));
        }
        if md.mode() & 0o022 != 0 {
            return Err(std::io::Error::other(format!("{} is group or world writable", path.display())));
        }
        if md.len() > max {
            return Err(std::io::Error::other(format!("{} is larger than {max} bytes", path.display())));
        }
        let mut s = String::new();
        f.take(max + 1).read_to_string(&mut s)?;
        Ok(s)
    }

    /// Atomic rewrite: temp file `O_EXCL|O_NOFOLLOW` 0600, forced to 0644
    /// root:root (explicit chmod/chown, independent of umask), fsync, rename.
    /// Refuses a non-regular destination (symlink...).
    pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
        if let Ok(md) = fs::symlink_metadata(path) {
            if !md.file_type().is_file() {
                return Err(std::io::Error::other(format!("{} is not a regular file (symlink?)", path.display())));
            }
        }
        let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".akm-tmp");
        let tmp = path.with_file_name(name);
        let open_tmp = || OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp);
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

    /// Removes a regular file (absent = fine, symlink = removed itself, never
    /// followed).
    pub fn remove(path: &Path) -> std::io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

pub mod params {
    //! `hid_apple` parameters: whitelist (akm-core), sysfs, modprobe.d.
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::Path;

    use crate::keymap::{kernel_param, parse_param_value};

    use crate::fsutil;

    /// New content of the modprobe file: sets each `name=value` token of the
    /// `options hid_apple ...` lines (first occurrence replaced, duplicates
    /// dropped), or appends one line. Other lines untouched.
    pub fn merge(existing: &str, set: &[(&str, i32)]) -> String {
        let mut out = String::new();
        let mut done = false;
        for line in existing.lines() {
            let mut toks = line.split_whitespace();
            if toks.next() == Some("options") && toks.next() == Some("hid_apple") {
                let mut seen = vec![false; set.len()];
                let mut parts: Vec<String> = Vec::new();
                for t in line.split_whitespace() {
                    match set.iter().position(|(n, _)| t.split_once('=').is_some_and(|(k, _)| k == *n)) {
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
            out.push_str(&format!("options hid_apple {}\n", opts.join(" ")));
        }
        out
    }

    /// Strict `NAME=VALUE` against the whitelist.
    pub fn parse_assignment(s: &str) -> Result<(&'static str, i32), String> {
        let (n, v) = s.split_once('=').ok_or_else(|| format!("expected NAME=VALUE, got {s:?}"))?;
        let p = kernel_param(n).ok_or_else(|| format!("parameter {n:?} not allowed"))?;
        let v = parse_param_value(p, v).map_err(|e| e.to_string())?;
        Ok((p.name, v))
    }

    pub fn persist(conf: &Path, set: &[(&str, i32)]) -> std::io::Result<()> {
        let existing = fsutil::read_regular(conf, 64 * 1024)?.unwrap_or_default();
        fsutil::atomic_write(conf, merge(&existing, set).as_bytes())
    }

    pub fn write_sysfs(dir: &Path, name: &str, v: i32) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        // No create: the parameter must exist (hid_apple loaded).
        let mut f = OpenOptions::new().write(true).custom_flags(libc::O_NOFOLLOW).open(dir.join(name))?;
        f.write_all(format!("{v}\n").as_bytes())
    }
}

pub mod keymap_install {
    //! Install / remove / roll back the hwdb file of the manual key mapping.
    //! The source is a FIXED path in the requesting user's runtime directory
    //! (`/run/user/$PKEXEC_UID/apple-kb-monitor/keymap.hwdb`), never a path
    //! from the command line; its content is validated line by line against
    //! the whitelist of [`crate::keymap::parse_hwdb`] and re-rendered
    //! canonically, so only `KEYBOARD_KEY_` lines for Apple aluminium
    //! keyboards can ever reach `/etc`.
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
        pub fn for_uid(uid: u32) -> Paths {
            Paths::new(source_for_uid(uid), PathBuf::from(TARGET))
        }

        pub fn new(source: PathBuf, target: PathBuf) -> Paths {
            let mut b = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            b.push(".akm-bak"); // not *.hwdb: systemd-hwdb ignores it
            Paths { source, backup: target.with_file_name(b), target }
        }
    }

    /// Where `akmctl` / the daemon put the file to install.
    pub fn source_for_uid(uid: u32) -> PathBuf {
        PathBuf::from(format!("/run/user/{uid}/apple-kb-monitor/keymap.hwdb"))
    }

    /// `PKEXEC_UID` (set by pkexec itself): digits only.
    pub fn parse_uid(s: &str) -> Result<u32, String> {
        if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
            return Err(format!("bad PKEXEC_UID {s:?}"));
        }
        s.parse().map_err(|_| format!("bad PKEXEC_UID {s:?}"))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Verb {
        Install,
        Remove,
        Rollback,
    }

    pub fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Verb, String> {
        match args.iter().map(AsRef::as_ref).collect::<Vec<_>>().as_slice() {
            ["install"] => Ok(Verb::Install),
            ["remove"] => Ok(Verb::Remove),
            ["rollback"] => Ok(Verb::Rollback),
            _ => Err("usage: akm-keymap-helper install|remove|rollback (the file is read from /run/user/$PKEXEC_UID/apple-kb-monitor/keymap.hwdb)".into()),
        }
    }

    /// Runs the external commands (`systemd-hwdb update`, `udevadm trigger`).
    pub trait Runner {
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
                Err(format!("{} failed ({st})", argv.join(" ")))
            }
        }
    }

    fn reload(r: &dyn Runner) -> Result<(), String> {
        r.run(&[SYSTEMD_HWDB, "update"])?;
        r.run(&[UDEVADM, "trigger", "--settle", "--subsystem-match=input", "--action=change"])
    }

    fn io(e: std::io::Error) -> String {
        e.to_string()
    }

    /// Installed records (an unparsable file counts as none: it is replaced).
    fn installed(path: &Path) -> Result<(Option<String>, Option<Vec<HwdbRecord>>), String> {
        Ok(match fsutil::read_regular(path, MAX_HWDB as u64).map_err(io)? {
            None => (None, None),
            Some(s) => (Some(s.clone()), parse_hwdb(&s).ok().map(|(_, r)| r)),
        })
    }

    /// Outcome message for the user.
    pub fn run(verb: Verb, p: &Paths, uid: u32, r: &dyn Runner) -> Result<String, String> {
        match verb {
            Verb::Install => install(p, uid, r),
            Verb::Remove => remove(p, r),
            Verb::Rollback => rollback(p, r),
        }
    }

    fn install(p: &Paths, uid: u32, r: &dyn Runner) -> Result<String, String> {
        let src = fsutil::read_user_file(&p.source, uid, MAX_HWDB as u64).map_err(io)?;
        let (profile, new) = parse_hwdb(&src).map_err(|e| format!("{} refused: {e}", p.source.display()))?;
        if new.is_empty() {
            return Err("nothing to install (no KEYBOARD_KEY_ line); use `remove` to go back to the kernel mapping".into());
        }
        let (old_text, old) = installed(&p.target)?;
        let content = render_hwdb(profile.as_deref().unwrap_or("unknown"), &transition(&old.unwrap_or_default(), &new));
        if let Some(t) = old_text {
            fsutil::atomic_write(&p.backup, t.as_bytes()).map_err(io)?;
        }
        fsutil::atomic_write(&p.target, content.as_bytes()).map_err(io)?;
        reload(r)?;
        Ok(format!("{} installed ({} model(s)); previous file kept as {}", p.target.display(), new.len(), p.backup.display()))
    }

    fn remove(p: &Paths, r: &dyn Runner) -> Result<String, String> {
        let (old_text, old) = installed(&p.target)?;
        let Some(text) = old_text else {
            return Ok("nothing installed: the kernel mapping is already in use".into());
        };
        fsutil::atomic_write(&p.backup, text.as_bytes()).map_err(io)?;
        // Kernel defaults written once (EVIOCSKEYCODE outlives the file),
        // then the file goes.
        let restore = transition(&old.unwrap_or_default(), &[]);
        if !restore.is_empty() {
            fsutil::atomic_write(&p.target, render_hwdb("restore-defaults", &restore).as_bytes()).map_err(io)?;
            reload(r)?;
        }
        fsutil::remove(&p.target).map_err(io)?;
        r.run(&[SYSTEMD_HWDB, "update"])?;
        Ok(format!("{} removed (kept as {}); kernel mapping restored", p.target.display(), p.backup.display()))
    }

    fn rollback(p: &Paths, r: &dyn Runner) -> Result<String, String> {
        let bak = fsutil::read_regular(&p.backup, MAX_HWDB as u64).map_err(io)?.ok_or("no previous mapping saved")?;
        let (profile, prev) = parse_hwdb(&bak).map_err(|e| format!("{} refused: {e}", p.backup.display()))?;
        let (cur_text, cur) = installed(&p.target)?;
        let content = render_hwdb(profile.as_deref().unwrap_or("unknown"), &transition(&cur.unwrap_or_default(), &prev));
        match cur_text {
            Some(t) => fsutil::atomic_write(&p.backup, t.as_bytes()).map_err(io)?,
            None => fsutil::remove(&p.backup).map_err(io)?,
        }
        fsutil::atomic_write(&p.target, content.as_bytes()).map_err(io)?;
        reload(r)?;
        Ok(format!("previous mapping restored in {}", p.target.display()))
    }
}

#[cfg(test)]
mod install_tests {
    //! The helper in a fake `/etc` (temp dir), with a recording runner.
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
        let p = Paths::new(d.join("run/keymap.hwdb"), d.join("etc/udev/hwdb.d/90-apple-kb-monitor.hwdb"));
        (d, p)
    }

    fn write_src(p: &Paths, s: &str) {
        let _ = fs::remove_file(&p.source);
        fs::write(&p.source, s).unwrap();
        fs::set_permissions(&p.source, fs::Permissions::from_mode(0o644)).unwrap();
    }

    fn rec(keys: &[(u32, u16)]) -> Vec<HwdbRecord> {
        vec![HwdbRecord { pid: 0x0256, keys: keys.iter().copied().collect() }]
    }

    #[test]
    fn install_rollback_remove_cycle() {
        let (d, p) = setup("cycle");
        let r = Rec::default();
        // 1st install: F6 → F13
        write_src(&p, &render_hwdb("a", &rec(&[(0x7003f, 183)])));
        run(Verb::Install, &p, uid(), &r).unwrap();
        let t1 = fs::read_to_string(&p.target).unwrap();
        assert!(t1.contains(" KEYBOARD_KEY_7003f=f13\n"), "{t1}");
        assert_eq!(fs::metadata(&p.target).unwrap().mode() & 0o7777, 0o644);
        assert!(!p.backup.exists());
        assert_eq!(
            *r.0.borrow(),
            ["/usr/bin/systemd-hwdb update", "/usr/bin/udevadm trigger --settle --subsystem-match=input --action=change"]
        );
        // 2nd install: Eject → Delete; F6 dropped → explicit default f6, backup = 1st
        write_src(&p, &render_hwdb("b", &rec(&[(SC_EJECT, 111)])));
        run(Verb::Install, &p, uid(), &r).unwrap();
        let t2 = fs::read_to_string(&p.target).unwrap();
        assert!(t2.contains(" KEYBOARD_KEY_7003f=f6\n") && t2.contains(" KEYBOARD_KEY_c00b8=delete\n"), "{t2}");
        assert_eq!(fs::read_to_string(&p.backup).unwrap(), t1);
        // rollback: back to F6 → F13, Eject restored to its default
        run(Verb::Rollback, &p, uid(), &r).unwrap();
        let t3 = fs::read_to_string(&p.target).unwrap();
        assert!(t3.contains("=f13\n") && t3.contains(" KEYBOARD_KEY_c00b8=ejectcd\n"), "{t3}");
        assert_eq!(fs::read_to_string(&p.backup).unwrap(), t2);
        // remove: defaults applied once, then file gone, hwdb rebuilt
        r.0.borrow_mut().clear();
        let m = run(Verb::Remove, &p, uid(), &r).unwrap();
        assert!(m.contains("removed"));
        assert!(!p.target.exists());
        assert_eq!(fs::read_to_string(&p.backup).unwrap(), t3);
        assert_eq!(r.0.borrow().len(), 3, "update + trigger with defaults, then update");
        // nothing installed: remove is a no-op
        assert!(run(Verb::Remove, &p, uid(), &r).unwrap().contains("nothing installed"));
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
        // wrong owner (uid+1), symlinked source, group-writable, hard link
        write_src(&p, &render_hwdb("a", &rec(&[(0x7003f, 183)])));
        assert!(run(Verb::Install, &p, uid() + 1, &r).unwrap_err().contains("not owned"));
        fs::set_permissions(&p.source, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r).unwrap_err().contains("writable"));
        fs::set_permissions(&p.source, fs::Permissions::from_mode(0o644)).unwrap();
        fs::hard_link(&p.source, d.join("run/link")).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r).unwrap_err().contains("hard links"));
        fs::remove_file(d.join("run/link")).unwrap();
        let real = d.join("run/real.hwdb");
        fs::rename(&p.source, &real).unwrap();
        symlink(&real, &p.source).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r).is_err(), "symlinked source");
        assert!(!p.target.exists() && r.0.borrow().is_empty());
        // symlink at the destination: refused, the victim untouched
        fs::remove_file(&p.source).unwrap();
        fs::rename(&real, &p.source).unwrap();
        let victim = d.join("victim");
        fs::write(&victim, "keep\n").unwrap();
        symlink(&victim, &p.target).unwrap();
        assert!(run(Verb::Install, &p, uid(), &r).is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep\n");
        // rollback without backup
        fs::remove_file(&p.target).unwrap();
        assert!(run(Verb::Rollback, &p, uid(), &r).unwrap_err().contains("no previous"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn args_and_uid_are_strict() {
        assert_eq!(parse_args(&["install"]), Ok(Verb::Install));
        for bad in [&[][..], &["install", "/tmp/x"], &["install", "--force"], &["--help"], &["../install"]] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_uid("1000"), Ok(1000));
        for bad in ["", "01000", "-1", "1000 ", "abc", "99999999999"] {
            assert!(parse_uid(bad).is_err(), "{bad:?}");
        }
        assert_eq!(source_for_uid(1000), PathBuf::from("/run/user/1000/apple-kb-monitor/keymap.hwdb"));
        assert_eq!(Paths::for_uid(1).backup, PathBuf::from("/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb.akm-bak"));
    }
}
