//! #114: the launcher is translated. Every translatable key of the .desktop
//! file (Name, GenericName, Comment, Keywords) has a French variant, and the
//! French window title is the French `Name`.

const DESKTOP: &str = include_str!("../../../com.agenceapi.AppleKbMonitor.desktop");

fn value<'a>(key: &str) -> Option<&'a str> {
    DESKTOP
        .lines()
        .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

#[test]
fn every_translatable_key_has_a_french_variant() {
    for key in ["Name", "GenericName", "Comment", "Keywords"] {
        let en = value(key).unwrap_or_else(|| panic!("{key} missing"));
        let fr = value(&format!("{key}[fr]")).unwrap_or_else(|| panic!("{key}[fr] missing"));
        assert!(!fr.is_empty());
        assert_ne!(en, fr, "{key}[fr] is the English text");
    }
    assert_eq!(value("Name[fr]"), Some("Moniteur de clavier Apple"));
    assert!(value("Keywords[fr]").unwrap().ends_with(';'), "Keywords is a ;-list");
}

#[test]
fn the_application_identity_is_not_translated() {
    assert_eq!(value("StartupWMClass"), Some("com.agenceapi.AppleKbMonitor"));
    assert_eq!(value("Exec"), Some("apihub-app"));
}
