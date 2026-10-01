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

/// Invisible characters that could spoof or hide a name (bidi overrides,
/// zero-width, BOM, line/paragraph separators).
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
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
    fn length_limits() {
        assert!(validate(&"a".repeat(MAX_CHARS)).is_ok());
        assert_eq!(validate(&"a".repeat(MAX_CHARS + 1)), Err(AliasError::TooLong));
        // 64 chars of 4 bytes = 256 bytes > 248
        assert_eq!(validate(&"\u{1F3B9}".repeat(64)), Err(AliasError::TooLong));
        assert!(validate(&"\u{e9}".repeat(64)).is_ok());
    }
}
