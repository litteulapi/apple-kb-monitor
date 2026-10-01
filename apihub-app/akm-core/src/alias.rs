//! User-chosen name of a keyboard (the BlueZ `Device1.Alias`, #141).
//!
//! The alias lives on the computer only: BlueZ stores it next to the pairing
//! data. It is *not* the name stored in the keyboard (Feature reports
//! 0x51-0x53), which this crate never writes (docs/RENOMMER-CLAVIER.md).

/// Longest alias, in characters.
pub const MAX_CHARS: usize = 64;
/// Longest alias, in UTF-8 bytes (BlueZ / HCI local name limit is 248).
pub const MAX_BYTES: usize = 248;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasError {
    TooLong,
    /// Control, line/paragraph separator or invisible formatting character.
    BadChar(char),
    /// Starts with `-`: would be read as an option by external dialogs (#207).
    LeadingDash,
}

impl std::fmt::Display for AliasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AliasError::TooLong => {
                write!(f, "name too long (max {MAX_CHARS} characters, {MAX_BYTES} bytes)")
            }
            AliasError::LeadingDash => f.write_str("name must not start with '-'"),
            AliasError::BadChar(c) => {
                write!(f, "name contains a forbidden character (U+{:04X})", u32::from(*c))
            }
        }
    }
}

impl std::error::Error for AliasError {}

/// Invisible, formatting or unassigned characters that could spoof or hide a
/// name. Covers the Unicode general categories Cf (format), Zl, Zp (line and
/// paragraph separators), Co (private use) and Cn (unassigned: the ranges
/// below stand for the non-characters and the stable unassigned blocks), plus
/// the "blank" fillers that are letters or symbols (Hangul fillers, Braille
/// blank) and the variation selectors / combining grapheme joiner.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'               // soft hyphen (Cf)
            | '\u{034F}'         // combining grapheme joiner
            | '\u{061C}'         // arabic letter mark (Cf)
            | '\u{115F}'..='\u{1160}' // hangul choseong/jungseong fillers
            | '\u{17B4}'..='\u{17B5}' // khmer inherent vowels (Cf)
            | '\u{180B}'..='\u{180F}' // mongolian free variation selectors, vowel sep.
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}' // word joiner .. deprecated formatting (incl. unassigned 2065)
            | '\u{2800}'         // braille pattern blank
            | '\u{3164}'         // hangul filler
            | '\u{E000}'..='\u{F8FF}' // private use (Co)
            | '\u{FE00}'..='\u{FE0F}' // variation selectors
            | '\u{FEFF}'
            | '\u{FFA0}'         // halfwidth hangul filler
            | '\u{FFF0}'..='\u{FFFB}' // unassigned + interlinear annotation (Cf)
            | '\u{FFFE}'..='\u{FFFF}' // non-characters
            | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
            | '\u{1D173}'..='\u{1D17A}' // musical formatting
            | '\u{E0000}'..='\u{E0FFF}' // tags + variation selectors supplement
            | '\u{F0000}'..='\u{10FFFF}' // supplementary private use planes (Co)
    )
}

/// Validate a requested alias. Surrounding whitespace is removed. An empty
/// result (`""`, `"   "`) is valid and means "reset to the keyboard's own
/// name" (BlueZ: an empty alias restores the remote name).
///
/// Input is a `&str`, hence already valid UTF-8; D-Bus strings cannot hold NUL.
pub fn validate(input: &str) -> Result<String, AliasError> {
    let name = input.trim();
    if let Some(c) = name.chars().find(|c| c.is_control() || is_invisible(*c)) {
        return Err(AliasError::BadChar(c));
    }
    if name.starts_with('-') {
        return Err(AliasError::LeadingDash);
    }
    if name.chars().count() > MAX_CHARS || name.len() > MAX_BYTES {
        return Err(AliasError::TooLong);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_trims() {
        assert_eq!(validate("Clavier de maria #1").unwrap(), "Clavier de maria #1");
        assert_eq!(validate("  Bureau \u{e9}t\u{e9} \u{1F3B9} ").unwrap(), "Bureau \u{e9}t\u{e9} \u{1F3B9}");
    }

    #[test]
    fn empty_means_reset() {
        assert_eq!(validate("").unwrap(), "");
        assert_eq!(validate(" \t ").unwrap(), "");
    }

    #[test]
    fn rejects_control_and_invisible() {
        for bad in ["a\nb", "a\tb", "a\0b", "\u{7f}", "x\u{85}y", "a\u{202E}b", "a\u{200B}b", "a\u{2028}b", "\u{FEFF}a"] {
            assert!(matches!(validate(bad), Err(AliasError::BadChar(_))), "{bad:?}");
        }
    }

    #[test]
    fn rejects_leading_dash() {
        for bad in ["--version", "-x", "  --textbox /etc/passwd"] {
            assert_eq!(validate(bad), Err(AliasError::LeadingDash), "{bad:?}");
        }
        assert!(validate("a-b").is_ok());
    }

    #[test]
    fn rejects_bidi_fillers_and_format_characters() {
        for c in [
            '\u{061C}', '\u{3164}', '\u{FFA0}', '\u{2800}', '\u{00AD}', '\u{180E}', '\u{034F}',
            '\u{2065}', '\u{E0041}', '\u{115F}', '\u{17B4}', '\u{FE0F}', '\u{FFF9}', '\u{E000}',
            '\u{FFFF}', '\u{F0000}', '\u{206A}', '\u{1D173}',
        ] {
            let s = format!("a{c}b");
            assert_eq!(validate(&s), Err(AliasError::BadChar(c)), "U+{:04X}", u32::from(c));
            assert_eq!(validate(&c.to_string()), Err(AliasError::BadChar(c)));
        }
    }

    #[test]
    fn keeps_legitimate_non_ascii_names() {
        for ok in ["\u{e9}t\u{e9}", "\u{4e2d}\u{6587}", "\u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629}", "\u{1F3B9}", "\u{1100}\u{1161}"] {
            assert!(validate(ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn length_limits() {
        assert!(validate(&"a".repeat(MAX_CHARS)).is_ok());
        assert_eq!(validate(&"a".repeat(MAX_CHARS + 1)), Err(AliasError::TooLong));
        // 64 chars of 4 bytes = 256 bytes > 248
        assert_eq!(validate(&"\u{1F3B9}".repeat(64)), Err(AliasError::TooLong));
        assert!(validate(&"\u{e9}".repeat(64)).is_ok());
    }
}
