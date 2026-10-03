//! Settings of the window read from the `[ui]` section of the daemon's
//! `config.toml` (`akm_core::config::default_path()`), **read-only**:
//!
//! ```toml
//! [ui]
//! crt_effects = true   # scanlines, vignette and glow of the window
//! ```
//!
//! The window never writes this file. It is parsed by `akm_core::config`,
//! the daemon's own parser, so both read it with the same rules (and the
//! daemon does not warn about `[ui]`).

/// What the window reads from `[ui]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSettings {
    /// Scanlines, vignette and glow. Default: on.
    pub crt_effects: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { crt_effects: true }
    }
}

pub fn parse(content: &str) -> UiSettings {
    parse_with_warnings(content).0
}

/// The settings and the warnings about `[ui]` (a value of the wrong type is
/// ignored: say so, #283). Warnings of the other sections are the daemon's.
pub fn parse_with_warnings(content: &str) -> (UiSettings, Vec<String>) {
    let (cfg, warnings) = akm_core::config::parse(content);
    let ui = warnings
        .into_iter()
        .filter(|w| w.contains("crt_effects") || w.contains("[ui]"))
        .collect();
    (
        UiSettings {
            crt_effects: cfg.ui_crt_effects,
        },
        ui,
    )
}

/// Settings of this session; a missing or unreadable file gives the defaults.
pub fn load() -> UiSettings {
    let path = akm_core::config::default_path();
    let Ok(c) = std::fs::read_to_string(&path) else {
        return UiSettings::default();
    };
    let (s, warnings) = parse_with_warnings(&c);
    for w in warnings {
        eprintln!(
            "[apihub] {}: {w} (expected: crt_effects = true or false)",
            path.display()
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crt_effects_default_on_and_read_from_the_ui_section_only() {
        assert!(parse("").crt_effects);
        assert!(parse("[alerts]\nenabled = false\n").crt_effects);
        assert!(!parse("[ui]\ncrt_effects = false\n").crt_effects);
        assert!(!parse("\u{feff}[ui]\n  crt_effects=false   # no scanlines\n").crt_effects);
        assert!(parse("[ui]\ncrt_effects = true\n[alerts]\ncrt_effects = false\n").crt_effects);
        // Another section, a comment or a wrong type never switches it off.
        assert!(parse("[display]\ncrt_effects = false\n").crt_effects);
        assert!(parse("[ui]\n# crt_effects = false\n").crt_effects);
        assert!(parse("[ui]\ncrt_effects = \"false\"\n").crt_effects);
        assert!(parse("[ui]\ncrt_effects = 0\n").crt_effects);
        assert!(!parse("[ui]\ncrt_effects = true\ncrt_effects = false\n").crt_effects);
    }

    /// #283: a value of the wrong type is ignored, but not silently.
    #[test]
    fn a_wrong_value_of_crt_effects_is_reported() {
        let (s, w) = parse_with_warnings("[ui]\ncrt_effects = \"false\"\n");
        assert!(s.crt_effects);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("crt_effects"), "{w:?}");
        assert!(parse_with_warnings("[ui]\ncrt_effects = false\n")
            .1
            .is_empty());
    }
}
