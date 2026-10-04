//! the keyboard name and any external text go into the notification body.

use std::time::Instant;

use akm_core::alerts::Urgency;
use akm_core::recovery::Notice;
use apple_kb_monitord::notify::{
    escape_markup, firmware_notification, generic_notification, notice_notification,
    removed_notification, Event,
};
use apple_kb_monitord::repair::notice_text;

const EVIL: &str = r#"<a href="http://x.invalid/">Keyboard</a><img src="file:///e">"#;

fn assert_inert(body: &str) {
    assert!(
        !body.contains('<') && !body.contains('>') && !body.contains('"'),
        "markup survived: {body}"
    );
    assert!(
        body.contains("&lt;a href=&quot;http://x.invalid/&quot;&gt;Keyboard&lt;/a&gt;"),
        "the name is shown, escaped: {body}"
    );
}

#[test]
fn the_vector_is_still_a_valid_alias() {
    assert!(akm_core::alias::validate(EVIL).is_ok());
}

#[test]
fn removed_notification_escapes_the_keyboard_name() {
    assert_inert(&removed_notification(EVIL).body);
}

#[test]
fn repair_notices_escape_the_keyboard_name() {
    let now = Instant::now();
    let (summary, body, urgency) = notice_text(&Notice::Unreachable { since: now }, EVIL, now);
    let n = notice_notification(Event::KeyboardUnreachable, &summary, &body, urgency);
    assert_inert(&n.body);
}

#[test]
fn firmware_and_generic_bodies_are_escaped_too() {
    let n = firmware_notification(EVIL, "1.0");
    assert_inert(&n.body);
    let n = generic_notification("x", EVIL, "dialog-error", Urgency::Normal, true);
    assert_inert(&n.body);
}

#[test]
fn escape_is_exact_and_leaves_plain_text_alone() {
    assert_eq!(escape_markup(r#"a&b<c>"d"#), "a&amp;b&lt;c&gt;&quot;d");
    assert_eq!(
        escape_markup("Alice's keyboard #1 \u{2014} 99 % \u{ab} ok \u{bb}"),
        "Alice's keyboard #1 \u{2014} 99 % \u{ab} ok \u{bb}"
    );
    let n = removed_notification("Magic Keyboard");
    assert!(
        n.body.contains("\u{201c}Magic Keyboard\u{201d}"),
        "{}",
        n.body
    );
}
