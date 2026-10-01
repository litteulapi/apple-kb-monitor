//! AUDIT FINAL (2026-10-01, #257) — le nom du clavier (alias BlueZ, modifiable
//! par toute application de la session via `SetAlias`, ou nom distant annoncé
//! par l'appareil) et tout texte externe entrent dans le corps des
//! notifications. À l'origine ce test prouvait l'absence d'échappement ; il
//! prouve maintenant que chaque constructeur échappe le corps. Aucune
//! notification réelle n'est envoyée : seule la construction est examinée.

use std::time::Instant;

use akm_core::alerts::Urgency;
use akm_core::recovery::Notice;
use apple_kb_monitord::notify::{
    escape_markup, firmware_notification, generic_notification, notice_notification,
    removed_notification, Event, Lang,
};
use apple_kb_monitord::repair::notice_text;

// 59 caractères : sous la limite de 64 de `alias::validate`.
const EVIL: &str = r#"<a href="http://x.invalid/">Clavier</a><img src="file:///e">"#;

fn assert_inert(body: &str) {
    assert!(
        !body.contains('<') && !body.contains('>') && !body.contains('"'),
        "markup survived: {body}"
    );
    assert!(
        body.contains("&lt;a href=&quot;http://x.invalid/&quot;&gt;Clavier&lt;/a&gt;"),
        "the name is shown, escaped: {body}"
    );
}

#[test]
fn the_vector_is_still_a_valid_alias() {
    // Le validateur accepte ce nom : la protection doit être à l'affichage.
    assert!(akm_core::alias::validate(EVIL).is_ok());
}

#[test]
fn removed_notification_escapes_the_keyboard_name() {
    for lang in [Lang::Fr, Lang::En] {
        assert_inert(&removed_notification(EVIL, lang).body);
    }
}

#[test]
fn repair_notices_escape_the_keyboard_name() {
    let now = Instant::now();
    for fr in [true, false] {
        let (summary, body, urgency) =
            notice_text(&Notice::Unreachable { since: now }, EVIL, now, fr);
        let n = notice_notification(Event::KeyboardUnreachable, &summary, &body, urgency);
        assert_inert(&n.body);
    }
}

#[test]
fn firmware_and_generic_bodies_are_escaped_too() {
    let n = firmware_notification(EVIL, "1.0", Lang::En);
    assert_inert(&n.body);
    let n = generic_notification("x", EVIL, "dialog-error", Urgency::Normal, true);
    assert_inert(&n.body);
}

#[test]
fn escape_is_exact_and_leaves_plain_text_alone() {
    assert_eq!(escape_markup(r#"a&b<c>"d"#), "a&amp;b&lt;c&gt;&quot;d");
    assert_eq!(
        escape_markup("Clavier de alice #1 \u{2014} 99 % « ok »"),
        "Clavier de alice #1 \u{2014} 99 % « ok »"
    );
    // plain body: unchanged by construction
    let n = removed_notification("Magic Keyboard", Lang::En);
    assert!(
        n.body.contains("\u{201c}Magic Keyboard\u{201d}"),
        "{}",
        n.body
    );
}
