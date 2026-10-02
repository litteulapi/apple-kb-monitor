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
    let (cfg, _warnings) = akm_core::config::parse(content);
    UiSettings {
        crt_effects: cfg.ui_crt_effects,
    }
}

/// Settings of this session; a missing or unreadable file gives the defaults.
pub fn load() -> UiSettings {
    std::fs::read_to_string(akm_core::config::default_path())
        .map(|c| parse(&c))
        .unwrap_or_default()
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
}
