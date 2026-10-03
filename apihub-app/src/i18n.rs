//! Window translations (#114): English is the language of the source, French
//! comes from `i18n/fr.po` (gettext format, embedded in the binary).
//!
//! * [`tr`] translates a literal; [`trf`] translates a template whose `{}`
//!   (in order) or `{0}` `{1}` (by index) are filled with the arguments, in
//!   the order the target language needs.
//! * The language follows `LC_ALL` > `LC_MESSAGES` > `LANG` (first non-empty
//!   one); a value starting with `fr` gives French, anything else English.
//! * A text that has no entry stays in English (never empty, never a crash);
//!   the test `every_window_text_has_a_french_translation` makes that case
//!   fail the build instead: it scans the sources for every `tr("…")` and
//!   `trf("…", …)` literal.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

const FR_PO: &str = include_str!("../i18n/fr.po");

/// Language from the three locale variables.
pub fn lang_is_french(get: impl Fn(&str) -> Option<String>) -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| get(k))
        .find(|v| !v.is_empty())
        .is_some_and(|v| v.to_ascii_lowercase().starts_with("fr"))
}

/// French for this process (read once).
pub fn is_french() -> bool {
    // Unit tests of the window assert English texts whatever the developer's
    // locale; French is tested through `tr_in(true, ..)`.
    #[cfg(test)]
    return false;
    #[cfg(not(test))]
    is_french_env()
}

#[cfg(not(test))]
fn is_french_env() -> bool {
    static FR: OnceLock<bool> = OnceLock::new();
    *FR.get_or_init(|| lang_is_french(|k| std::env::var(k).ok()))
}

/// `msgid` -> `msgstr` of a gettext file (single or multi-line strings,
/// `\n \t \" \\` escapes, `#` comments; the header entry and untranslated
/// entries are skipped).
pub fn parse_po(po: &str) -> HashMap<String, String> {
    fn unquote(l: &str) -> String {
        let l = l.trim();
        let inner = l
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or("");
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        }
        out
    }
    let mut map = HashMap::new();
    let (mut id, mut st) = (String::new(), String::new());
    #[derive(PartialEq)]
    enum In {
        None,
        Id,
        Str,
    }
    let mut cur = In::None;
    let mut flush = |id: &mut String, st: &mut String| {
        if !id.is_empty() && !st.is_empty() {
            map.insert(std::mem::take(id), std::mem::take(st));
        }
        id.clear();
        st.clear();
    };
    for line in po.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            if line.is_empty() && cur == In::Str {
                flush(&mut id, &mut st);
                cur = In::None;
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("msgid ") {
            if cur == In::Str {
                flush(&mut id, &mut st);
            }
            id = unquote(r);
            cur = In::Id;
        } else if let Some(r) = line.strip_prefix("msgstr ") {
            st = unquote(r);
            cur = In::Str;
        } else if line.starts_with('"') {
            match cur {
                In::Id => id.push_str(&unquote(line)),
                In::Str => st.push_str(&unquote(line)),
                In::None => {}
            }
        }
    }
    flush(&mut id, &mut st);
    map
}

fn catalog() -> &'static HashMap<String, String> {
    static C: OnceLock<HashMap<String, String>> = OnceLock::new();
    C.get_or_init(|| parse_po(FR_PO))
}

/// `en` in the language of the session.
pub fn tr(en: &'static str) -> &'static str {
    tr_in(is_french(), en)
}

/// `en` in French (`fr`) or English; unknown texts stay English.
pub fn tr_in(fr: bool, en: &'static str) -> &'static str {
    if !fr {
        return en;
    }
    catalog().get(en).map_or(en, |s| s.as_str())
}

/// Fill `{}` (in order) and `{N}` (by index) of a template.
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0usize;
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        match tail.find('}') {
            Some(j) if tail[..j].chars().all(|c| c.is_ascii_digit()) => {
                let idx = if tail[..j].is_empty() {
                    let k = next;
                    next += 1;
                    k
                } else {
                    tail[..j].parse().unwrap_or(usize::MAX)
                };
                if let Some(a) = args.get(idx) {
                    out.push_str(&a.to_string());
                }
                rest = &tail[j + 1..];
            }
            _ => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Translated template filled with `args`.
pub fn trf(en: &'static str, args: &[&dyn Display]) -> String {
    fill(tr(en), args)
}

pub fn trf_in(fr: bool, en: &'static str, args: &[&dyn Display]) -> String {
    fill(tr_in(fr, en), args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn language_follows_lc_all_then_lc_messages_then_lang() {
        assert!(lang_is_french(env(&[("LANG", "fr_FR.UTF-8")])));
        assert!(!lang_is_french(env(&[
            ("LANG", "fr_FR.UTF-8"),
            ("LC_ALL", "en_US.UTF-8")
        ])));
        assert!(lang_is_french(env(&[
            ("LANG", "en_US.UTF-8"),
            ("LC_MESSAGES", "fr_BE")
        ])));
        assert!(lang_is_french(env(&[
            ("LC_ALL", ""),
            ("LANG", "fr_CA.UTF-8")
        ])));
        assert!(!lang_is_french(env(&[])), "English is the default");
        assert!(!lang_is_french(env(&[("LANG", "C")])));
    }

    #[test]
    fn po_parser_handles_multiline_escapes_and_untranslated() {
        let po = "msgid \"\"\nmsgstr \"Project-Id-Version: x\\n\"\n\n#. note\nmsgid \"A \\\"b\\\"\"\nmsgstr \"\"\n\"Un \\\"b\\\"\"\n\" suite\"\n\nmsgid \"Empty\"\nmsgstr \"\"\n";
        let m = parse_po(po);
        assert_eq!(m.get("A \"b\"").map(String::as_str), Some("Un \"b\" suite"));
        assert!(!m.contains_key("Empty") && !m.contains_key(""));
    }

    #[test]
    fn fill_orders_and_indexes() {
        assert_eq!(fill("a {} b {}", &[&1, &"x"]), "a 1 b x");
        assert_eq!(fill("b {1} a {0}", &[&"A", &"B"]), "b B a A");
        assert_eq!(fill("100 {}% {not}", &[&5]), "100 5% {not}");
        assert_eq!(fill("{}", &[]), "");
    }

    #[test]
    fn french_and_english_lookup() {
        assert_eq!(tr_in(false, "Keyboard"), "Keyboard");
        assert_eq!(tr_in(true, "Keyboard"), "Clavier");
        assert_eq!(
            tr_in(true, "not in the catalog at all"),
            "not in the catalog at all"
        );
    }

    /// Every literal given to `tr(` / `trf(` / `tr_in(` / `trf_in(` in the
    /// window sources has a French entry, different from the English text
    /// unless it is listed in `i18n/same-in-french.txt` (proper nouns, units).
    #[test]
    fn every_window_text_has_a_french_translation() {
        // The crate root that holds `src/` and `i18n/`: this file's
        // grandparent, so that the test also runs when `src/i18n.rs` is
        // included by `#[path]` from another crate (audit-ui).
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join(file!());
        let root = here.parent().and_then(Path::parent).unwrap();
        let same: Vec<String> = std::fs::read_to_string(root.join("i18n/same-in-french.txt"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        let cat = parse_po(FR_PO);
        let mut used = std::collections::HashSet::new();
        let mut missing = Vec::new();
        for e in std::fs::read_dir(root.join("src")).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "rs") || p.ends_with("i18n.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            for lit in literals(&src) {
                used.insert(lit.clone());
                match cat.get(&lit) {
                    None => missing.push(format!(
                        "{}: {lit:?}",
                        p.file_name().unwrap().to_string_lossy()
                    )),
                    Some(fr) if *fr == lit && !same.contains(&lit) => {
                        missing.push(format!("identical to English: {lit:?}"))
                    }
                    _ => {}
                }
            }
        }
        assert!(
            missing.is_empty(),
            "untranslated window texts:\n{}",
            missing.join("\n")
        );
        // And no orphan entry left in the catalog.
        let mut orphans: Vec<&String> = cat.keys().filter(|k| !used.contains(*k)).collect();
        orphans.sort();
        assert!(
            orphans.is_empty(),
            "catalog entries no source uses: {orphans:?}"
        );
        assert!(used.len() > 100, "scan found only {} texts", used.len());
    }

    /// String literals passed as first argument to the translation functions.
    fn literals(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        for f in ["tr(", "trf(", "tr_in(", "trf_in("] {
            let mut from = 0;
            while let Some(i) = src[from..].find(f) {
                let at = from + i;
                from = at + f.len();
                // a whole identifier: not `str(` or `my_tr(`
                if src[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    continue;
                }
                let mut args = &src[from..];
                if f.ends_with("_in(") {
                    // skip the first argument (`fr`)
                    let Some(c) = args.find(',') else { continue };
                    args = &args[c + 1..];
                }
                let args = args.trim_start();
                if let Some(rest) = args.strip_prefix('"') {
                    let mut s = String::new();
                    let mut it = rest.chars();
                    while let Some(c) = it.next() {
                        match c {
                            '"' => break,
                            '\\' => match it.next() {
                                Some('n') => s.push('\n'),
                                Some('"') => s.push('"'),
                                Some('\\') => s.push('\\'),
                                Some('u') => {
                                    // \u{XXXX}
                                    let mut h = String::new();
                                    for c in it.by_ref() {
                                        if c == '{' {
                                            continue;
                                        }
                                        if c == '}' {
                                            break;
                                        }
                                        h.push(c);
                                    }
                                    if let Some(ch) =
                                        u32::from_str_radix(&h, 16).ok().and_then(char::from_u32)
                                    {
                                        s.push(ch);
                                    }
                                }
                                Some(o) => s.push(o),
                                None => {}
                            },
                            c => s.push(c),
                        }
                    }
                    out.push(s);
                }
            }
        }
        out
    }
}
