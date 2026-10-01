//! AUDIT FINAL (2026-10-01) — le nom du clavier (alias BlueZ, modifiable par
//! toute application de la session via `SetAlias`, ou nom distant annoncé par
//! l'appareil) entre tel quel dans le corps des notifications. Le tray échappe
//! (`escape_markup`), les notifications non. Aucune notification réelle n'est
//! envoyée : seule la construction du texte est examinée.

use apple_kb_monitord::notify::{removed_notification, Lang};

#[test]
fn keyboard_name_markup_reaches_the_notification_body_unescaped() {
    // 59 caractères : sous la limite de 64 de `alias::validate`.
    let evil = r#"<a href="http://x.invalid/">Clavier</a><img src="file:///e">"#;
    // Le validateur d'alias accepte ce nom (ni caractère de contrôle, ni invisible).
    assert!(akm_core::alias::validate(evil).is_ok(), "alias refused: the vector is closed");
    let n = removed_notification(evil, Lang::Fr);
    assert!(
        n.body.contains("<a href=") && n.body.contains("<img src="),
        "body is escaped: {}",
        n.body
    );
    let n = removed_notification(evil, Lang::En);
    assert!(n.body.contains("<a href="), "{}", n.body);
}
