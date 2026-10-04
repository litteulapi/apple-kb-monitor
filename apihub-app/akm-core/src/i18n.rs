//! GNU gettext through glibc: English msgids, catalogs in `<dir>/<lang>/LC_MESSAGES/apple-kb-monitor.mo`.
//!
//! Until [`init`] is called every lookup returns the English text (tests stay deterministic).

use std::ffi::{c_char, c_ulong, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};

/// Installed catalogs; `AKM_LOCALEDIR` overrides it (tests, build tree).
pub const LOCALEDIR: &str = "/usr/share/locale";

const DOMAIN_C: &CStr = c"apple-kb-monitor";

static READY: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn bindtextdomain(domain: *const c_char, dir: *const c_char) -> *mut c_char;
    fn bind_textdomain_codeset(domain: *const c_char, codeset: *const c_char) -> *mut c_char;
    fn dgettext(domain: *const c_char, msgid: *const c_char) -> *mut c_char;
    fn dngettext(
        domain: *const c_char,
        one: *const c_char,
        many: *const c_char,
        n: c_ulong,
    ) -> *mut c_char;
}

/// Reads the locale from the environment and binds the catalog. Call once, first thing in `main`.
pub fn init() {
    let dir = std::env::var("AKM_LOCALEDIR")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| LOCALEDIR.to_string());
    bind(&dir);
}

/// For a program running as root: installed catalogs only (`AKM_LOCALEDIR` ignored), and
/// English unless every locale variable is a plain locale name (no path, no `..`).
pub fn init_privileged() {
    if locale_env_plain(|k| std::env::var_os(k)) {
        bind(LOCALEDIR);
    }
}

/// Every locale variable `bind` consults, `LC_CTYPE` included, is a plain locale name.
fn locale_env_plain(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LC_CTYPE", "LANG"]
        .into_iter()
        .filter_map(get)
        .all(|v| v.to_str().is_some_and(|s| s.split(':').all(plain_locale)))
}

/// `fr`, `fr_FR`, `fr_FR.UTF-8`, `sr_RS@latin`: letters, digits, `_ . - @` and nothing else.
#[must_use]
pub fn plain_locale(name: &str) -> bool {
    !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'@'))
}

fn bind(dir: &str) {
    let Ok(dir) = CString::new(dir) else { return };
    // Only messages and charset: numbers and dates keep the C conventions.
    // SAFETY: called before any other thread uses the locale; all pointers are valid C strings.
    unsafe {
        libc::setlocale(libc::LC_CTYPE, c"".as_ptr());
        libc::setlocale(libc::LC_MESSAGES, c"".as_ptr());
        bindtextdomain(DOMAIN_C.as_ptr(), dir.as_ptr());
        bind_textdomain_codeset(DOMAIN_C.as_ptr(), c"UTF-8".as_ptr());
    }
    READY.store(true, Ordering::Release);
}

fn owned(p: *const c_char, fallback: &str) -> String {
    if p.is_null() {
        return fallback.to_string();
    }
    // SAFETY: gettext returns a NUL-terminated string that lives as long as the process.
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .map_or_else(|_| fallback.to_string(), str::to_string)
}

/// Translation of `msgid` (the English text itself without catalog).
pub fn gettext(msgid: &str) -> String {
    if !READY.load(Ordering::Acquire) {
        return msgid.to_string();
    }
    let Ok(m) = CString::new(msgid) else {
        return msgid.to_string();
    };
    // SAFETY: valid C strings; the result is owned by gettext.
    owned(unsafe { dgettext(DOMAIN_C.as_ptr(), m.as_ptr()) }, msgid)
}

/// Plural form for `n`.
pub fn ngettext(one: &str, many: &str, n: u64) -> String {
    let english = if n == 1 { one } else { many };
    if !READY.load(Ordering::Acquire) {
        return english.to_string();
    }
    let (Ok(o), Ok(m)) = (CString::new(one), CString::new(many)) else {
        return english.to_string();
    };
    // SAFETY: valid C strings; the result is owned by gettext.
    owned(
        unsafe { dngettext(DOMAIN_C.as_ptr(), o.as_ptr(), m.as_ptr(), n as c_ulong) },
        english,
    )
}

/// Fills `{}`, `{0}` and `{name}` after the lookup; `{{` and `}}` are literal braces.
#[must_use]
pub fn fill(fmt: &str, args: &[(Option<&str>, String)]) -> String {
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut next = 0;
    let mut rest = fmt;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        let (true, Some(close)) = (tail.starts_with('{'), tail.find('}')) else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
            continue;
        };
        let key = &tail[1..close];
        let val = if key.is_empty() {
            next += 1;
            args.iter().filter(|a| a.0.is_none()).nth(next - 1)
        } else if let Ok(n) = key.parse::<usize>() {
            args.iter().filter(|a| a.0.is_none()).nth(n)
        } else {
            args.iter().find(|a| a.0 == Some(key))
        };
        match val {
            Some(v) => out.push_str(&v.1),
            None => out.push_str(&tail[..=close]),
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// `tr!("text")`, `tr!("{} keys", n)` or `tr!("{name} left", name = x)`: lookup, then fill.
#[macro_export]
macro_rules! tr {
    ($m:literal $(,)?) => {
        $crate::i18n::gettext($m)
    };
    ($m:literal, $($n:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::fill(&$crate::i18n::gettext($m), &[$((Some(stringify!($n)), ($v).to_string())),+])
    };
    ($m:literal, $($v:expr),+ $(,)?) => {
        $crate::i18n::fill(&$crate::i18n::gettext($m), &[$((None, ($v).to_string())),+])
    };
}

/// `N_!("text")`: marks a constant for xgettext and leaves it as is; translate it with
/// [`gettext`] where it is shown.
#[macro_export]
macro_rules! N_ {
    ($m:literal) => {
        $m
    };
}

/// `trn!("one key", "{} keys", n, n)`: plural lookup on `n`, then fill.
#[macro_export]
macro_rules! trn {
    ($one:literal, $many:literal, $n:expr $(,)?) => {
        $crate::i18n::ngettext($one, $many, ($n) as u64)
    };
    ($one:literal, $many:literal, $n:expr, $($n2:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::fill(&$crate::i18n::ngettext($one, $many, ($n) as u64), &[$((Some(stringify!($n2)), ($v).to_string())),+])
    };
    ($one:literal, $many:literal, $n:expr, $($v:expr),+ $(,)?) => {
        $crate::i18n::fill(&$crate::i18n::ngettext($one, $many, ($n) as u64), &[$((None, ($v).to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_init_the_english_text_comes_back() {
        assert_eq!(ngettext("one key", "{} keys", 1), "one key");
        assert_eq!(ngettext("one key", "{} keys", 2), "{} keys");
    }

    #[test]
    fn test_only_msgids_stay_out_of_the_catalog() {
        let pot = include_str!("../../../po/apple-kb-monitor.pot");
        assert!(
            !pot.contains("msgid \"one key\""),
            "update-po.sh must pass -k to xgettext"
        );
    }

    #[test]
    fn privileged_locale_names_are_plain() {
        assert!(plain_locale("fr_FR.UTF-8") && plain_locale("sr_RS@latin") && plain_locale(""));
        assert!(!plain_locale("../../tmp/x") && !plain_locale("/tmp/x") && !plain_locale("fr..x"));
    }

    #[test]
    fn privileged_env_vets_lc_ctype() {
        let env = |bad: &'static str| {
            move |k: &str| (k == bad).then(|| std::ffi::OsString::from("../../tmp/x"))
        };
        for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LC_CTYPE", "LANG"] {
            assert!(!locale_env_plain(env(var)), "{var}");
        }
        assert!(locale_env_plain(|_| Some("fr_FR.UTF-8".into())));
    }

    #[test]
    fn fill_positional_indexed_named_and_braces() {
        let a = |v: &str| (None, v.to_string());
        assert_eq!(fill("{} of {}", &[a("1"), a("2")]), "1 of 2");
        assert_eq!(fill("{1} before {0}", &[a("a"), a("b")]), "b before a");
        assert_eq!(fill("{name} left", &[(Some("name"), "x".into())]), "x left");
        assert_eq!(fill("{{literal}} {}", &[a("3")]), "{literal} 3");
        assert_eq!(fill("{missing} {", &[]), "{missing} {");
    }
}
