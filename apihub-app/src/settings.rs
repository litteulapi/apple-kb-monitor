//! Settings of the window read from the `[ui]` section of the daemon's
//! `config.toml` (`akm_core::config::default_path()`), **read-only**:
//!
//! ```toml
//! [ui]
//! crt_effects = true   # scanlines, vignette and glow of the window
//! ```
//!
//! The window never writes this file. `akm-core`'s typed configuration has
//! no `[ui]` section, so the two lines are read here, with the same rules as
//! the daemon for a TOML boolean; anything else keeps the default.

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

/// `line` without its trailing `#` comment (a `#` inside a string stays).
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

pub fn parse(content: &str) -> UiSettings {
    let mut out = UiSettings::default();
    let mut in_ui = false;
    for line in content.trim_start_matches('\u{feff}').lines() {
        let line = strip_comment(line).trim();
        if let Some(section) = line.strip_prefix('[') {
            in_ui = section.trim_end_matches(']').trim() == "ui";
            continue;
        }
        if !in_ui {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "crt_effects" {
                match value.trim() {
                    "true" => out.crt_effects = true,
                    "false" => out.crt_effects = false,
                    _ => {}
                }
            }
        }
    }
    out
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
