//! Tests ciblés de la reconnexion (`recovery`) : constantes, classification,
//! calendrier des tentatives, seuils d'« injoignable », accesseurs.
//! Chaque valeur attendue est recalculée à la main à partir des durées documentées.

use akm_core::recovery::*;
use std::time::{Duration, Instant};

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Clavier jamais connecté puis lien perdu (timeout) à `t`.
fn lost(t: Instant) -> Recovery {
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Timeout, t);
    r
}

#[test]
fn constants() {
    assert_eq!(MIN_SPACING, s(20));
    assert_eq!(BUSY_RETRY, s(30));
    assert_eq!(RESUME_GRACE, s(4));
    assert_eq!(LOSS_GRACE, s(20));
    assert_eq!(DORMANT_GRACE, s(300));
    assert_eq!(MEDIUM_PERIOD, s(300));
    assert_eq!(SLOW_PERIOD, s(900));
    assert_eq!(SLOW_AFTER, s(3600));
    assert_eq!(UNREACHABLE_AFTER, s(600));
    assert_eq!(UNREACHABLE_MIN_FAILURES, 3);
}

#[test]
fn health_text_roundtrip() {
    let all = [
        (Health::Unknown, "unknown"),
        (Health::Connected, "connected"),
        (Health::Dormant, "dormant"),
        (Health::Unreachable, "unreachable"),
        (Health::AuthFailed, "auth-failed"),
        (Health::Suspended, "suspended"),
    ];
    for (h, t) in all {
        assert_eq!(h.as_str(), t);
        assert_eq!(Health::parse(t), Some(h));
    }
    assert_eq!(Health::parse(""), None);
    assert_eq!(Health::parse("Connected"), None);
}

#[test]
fn disconnect_reason_names() {
    let all = [
        ("org.bluez.Reason.Timeout", DisconnectReason::Timeout, "timeout"),
        ("org.bluez.Reason.Local", DisconnectReason::Local, "local"),
        ("org.bluez.Reason.Remote", DisconnectReason::Remote, "remote"),
        ("org.bluez.Reason.Authentication", DisconnectReason::Authentication, "authentication"),
        ("org.bluez.Reason.Suspend", DisconnectReason::Suspend, "suspend"),
        ("org.bluez.Reason.Unknown", DisconnectReason::Unknown, "unknown"),
    ];
    for (n, r, t) in all {
        assert_eq!(DisconnectReason::from_bluez(n), r, "{n}");
        assert_eq!(DisconnectReason::from_bluez(n.rsplit('.').next().unwrap()), r);
        assert_eq!(r.as_str(), t);
    }
    assert_eq!(DisconnectReason::from_bluez("org.bluez.Reason.Whatever"), DisconnectReason::Unknown);
    assert_eq!(DisconnectReason::from_bluez(""), DisconnectReason::Unknown);
}

#[test]
fn classify_by_error_name() {
    use ConnectError as E;
    let by_name = [
        ("org.bluez.Error.AlreadyConnected", E::AlreadyConnected),
        ("org.bluez.Error.InProgress", E::Busy),
        ("org.bluez.Error.Busy", E::Busy),
        ("org.bluez.Error.AuthenticationFailed", E::Auth),
        ("org.bluez.Error.AuthenticationRejected", E::Auth),
        ("org.bluez.Error.AuthenticationTimeout", E::Auth),
        ("org.bluez.Error.NotReady", E::AdapterOff),
        ("org.freedesktop.DBus.Error.UnknownObject", E::DeviceGone),
        ("org.freedesktop.DBus.Error.UnknownMethod", E::DeviceGone),
        ("org.freedesktop.DBus.Error.ServiceUnknown", E::DeviceGone),
        ("org.bluez.Error.DoesNotExist", E::DeviceGone),
        ("Busy", E::Busy),
    ];
    for (n, want) in by_name {
        // Le nom l'emporte sur un message trompeur.
        assert_eq!(E::classify(n, "host is down"), want, "{n}");
        assert_eq!(E::classify(n, ""), want, "{n}");
    }
}

#[test]
fn classify_by_message_each_phrase() {
    use ConnectError as E;
    let neutral = "org.bluez.Error.Failed";
    let by_msg = [
        ("br-connection-key-missing", E::Auth),
        ("Authentication Failed", E::Auth),
        ("already-connected", E::AlreadyConnected),
        ("Already Connected", E::AlreadyConnected),
        ("Resource busy", E::Busy),
        ("operation in progress", E::Busy),
        ("InProgress", E::Busy),
        ("adapter-not-powered", E::AdapterOff),
        ("Adapter Not Ready", E::AdapterOff),
        ("br-connection-page-timeout", E::NoAnswer),
        ("br-connection-create-socket", E::NoAnswer),
        ("connect: Host is down", E::NoAnswer),
        ("br-connection-timeout", E::NoAnswer),
        ("br-connection-aborted-by-remote", E::NoAnswer),
        ("Connection refused", E::NoAnswer),
    ];
    for (m, want) in by_msg {
        assert_eq!(E::classify(neutral, m), want, "{m}");
    }
    assert_eq!(E::classify(neutral, "weird"), E::Other("weird".into()));
    assert_eq!(E::classify(neutral, "Weird Case"), E::Other("Weird Case".into()));
    assert_eq!(E::classify("", ""), E::Other(String::new()));
    // Priorité : l'authentification passe avant « refused » et « busy ».
    assert_eq!(E::classify(neutral, "authentication refused"), E::Auth);
    assert_eq!(E::classify(neutral, "busy and refused"), E::Busy);
}

#[test]
fn describe_texts() {
    use ConnectError as E;
    assert_eq!(E::NoAnswer.describe(), "no answer to the page (keyboard asleep or out of range)");
    assert_eq!(E::Busy.describe(), "BlueZ busy (connection already in progress)");
    assert_eq!(E::AlreadyConnected.describe(), "already connected");
    assert_eq!(E::Auth.describe(), "pairing refused (link key missing or rejected)");
    assert_eq!(E::AdapterOff.describe(), "adapter off or not ready");
    assert_eq!(E::DeviceGone.describe(), "device unknown to BlueZ (unpaired?)");
    assert_eq!(E::Other("boom".into()).describe(), "boom");
}

#[test]
fn journal_each_phrase() {
    use JournalKind as K;
    let cases = [
        ("connect to AA: Host is down (112)", Some(K::PageTimeout)),
        ("Page Timeout", Some(K::PageTimeout)),
        ("HIDP GET_REPORT request timed out", Some(K::GetReportTimeout)),
        ("Authentication failed", Some(K::Auth)),
        ("Key Missing", Some(K::Auth)),
        ("PIN or Key missing", Some(K::Auth)),
        ("Bonding failed", Some(K::Auth)),
        ("Connection reset by peer", Some(K::Refused)),
        ("Connection refused (111)", Some(K::Refused)),
        ("Unknown key ReverseServiceDiscovery for group General", Some(K::ConfigIgnored)),
        ("Unable set contents for /var/lib/bluetooth/x", Some(K::StorageError)),
        ("No space left on device", Some(K::StorageError)),
        ("Unknown key X", None),
        ("something for group Y", None),
        ("", None),
        ("battery level 55", None),
    ];
    for (m, want) in cases {
        assert_eq!(classify_journal(m), want, "{m}");
    }
    for (k, t) in [
        (K::PageTimeout, "page-timeout"),
        (K::GetReportTimeout, "get-report-timeout"),
        (K::Auth, "auth"),
        (K::Refused, "refused"),
        (K::ConfigIgnored, "config-ignored"),
        (K::StorageError, "storage-error"),
    ] {
        assert_eq!(k.as_str(), t);
    }
}

// ── machine d'états ─────────────────────────────────────────────────────────

#[test]
fn fresh_state_and_accessors() {
    let r = Recovery::default();
    assert_eq!(r.health(), Health::Unknown);
    assert_eq!((r.since(), r.attempts(), r.failures()), (None, 0, 0));
    assert_eq!((r.last_error(), r.last_reason(), r.next_deadline()), (None, None, None));
}

#[test]
fn start_variants() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.start(true, true, t);
    assert_eq!(r.health(), Health::Connected);
    assert_eq!(r.next_deadline(), None);

    let mut r = Recovery::new();
    r.start(false, true, t);
    assert_eq!(r.health(), Health::Dormant);
    assert_eq!(r.since(), Some(t));
    assert_eq!(r.next_deadline(), Some(t + RESUME_GRACE));
    assert!(r.poll(t + RESUME_GRACE - Duration::from_millis(1)).is_empty());
    assert_eq!(r.poll(t + RESUME_GRACE), vec![Action::Connect]);
    assert_eq!(r.attempts(), 1);
    assert_eq!(r.next_deadline(), None, "tentative en vol");
    assert!(r.poll(t + s(100)).is_empty(), "une seule tentative à la fois");

    let mut r = Recovery::new();
    r.start(false, false, t);
    assert_eq!(r.health(), Health::AuthFailed);
    assert_eq!(r.last_error(), Some("the keyboard is not paired with this computer"));
    assert_eq!(
        r.poll(t),
        vec![Action::Notify(Notice::RepairNeeded { why: "the keyboard is not paired with this computer".into() })]
    );
    assert!(r.poll(t + s(1000)).is_empty());
    assert_eq!(r.next_deadline(), None);
}

#[test]
fn loss_schedule_then_medium_period() {
    let t = Instant::now();
    let mut r = lost(t);
    assert_eq!(r.health(), Health::Dormant);
    assert_eq!(r.next_deadline(), Some(t + LOSS_GRACE));
    let mut now = t + LOSS_GRACE;
    // Écarts attendus entre une tentative et la suivante : 20, 40, 60, 120, puis 300.
    for gap in [20u64, 40, 60, 120, 300, 300] {
        assert_eq!(r.poll(now), vec![Action::Connect]);
        r.on_connect_result(Err(ConnectError::NoAnswer), now);
        assert_eq!(r.next_deadline(), Some(now + s(gap)), "écart {gap}");
        now += s(gap);
    }
    assert_eq!(r.attempts(), 6);
    assert_eq!(r.failures(), 6);
    assert_eq!(r.last_error(), Some("no answer to the page (keyboard asleep or out of range)"));
}

#[test]
fn slow_period_after_an_hour() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Remote, t); // Quiet : 300 s partout
    assert_eq!(r.next_deadline(), Some(t + DORMANT_GRACE));
    // Juste avant 1 h : période moyenne ; à 1 h pile : période lente.
    r.on_connect_result(Err(ConnectError::NoAnswer), t + SLOW_AFTER - s(1));
    assert_eq!(r.next_deadline(), Some(t + SLOW_AFTER - s(1) + MEDIUM_PERIOD));
    r.on_connect_result(Err(ConnectError::NoAnswer), t + SLOW_AFTER);
    assert_eq!(r.next_deadline(), Some(t + SLOW_AFTER + SLOW_PERIOD));
    // Cause « perdu » : le calendrier court est aussi remplacé par la période lente.
    let mut r = lost(t);
    r.on_connect_result(Err(ConnectError::NoAnswer), t + SLOW_AFTER);
    assert_eq!(r.next_deadline(), Some(t + SLOW_AFTER + SLOW_PERIOD));
}

#[test]
fn connect_result_ok_busy_other() {
    let t = Instant::now();
    let mut r = lost(t);
    assert_eq!(r.poll(t + LOSS_GRACE), vec![Action::Connect]);
    r.on_connect_result(Ok(()), t + s(25));
    assert_eq!(r.next_deadline(), Some(t + s(25) + MIN_SPACING));
    assert_eq!((r.failures(), r.last_error()), (0, None));
    assert_eq!(r.poll(t + s(44)), vec![]);
    assert_eq!(r.poll(t + s(45)), vec![Action::Connect]);
    r.on_connect_result(Err(ConnectError::Busy), t + s(50));
    assert_eq!(r.next_deadline(), Some(t + s(50) + BUSY_RETRY));
    assert_eq!(r.failures(), 0, "occupé n'est pas un échec");
    assert_eq!(r.last_error(), Some("BlueZ busy (connection already in progress)"));
    assert_eq!(r.poll(t + s(80)), vec![Action::Connect]);
    r.on_connect_result(Err(ConnectError::AlreadyConnected), t + s(81));
    assert_eq!(r.next_deadline(), Some(t + s(81) + BUSY_RETRY));
    assert_eq!(r.poll(t + s(111)), vec![Action::Connect]);
    r.on_connect_result(Err(ConnectError::Other("boom".into())), t + s(112));
    assert_eq!((r.failures(), r.last_error()), (1, Some("boom")));
}

#[test]
fn connect_result_auth_gone_adapter() {
    let t = Instant::now();
    let mut r = lost(t);
    r.poll(t + LOSS_GRACE);
    r.on_connect_result(Err(ConnectError::Auth), t + s(21));
    assert_eq!(r.health(), Health::AuthFailed);
    assert_eq!(
        r.poll(t + s(21)),
        vec![Action::Notify(Notice::RepairNeeded { why: "BlueZ reports the pairing as refused".into() })]
    );

    let mut r = lost(t);
    r.poll(t + LOSS_GRACE);
    r.on_connect_result(Err(ConnectError::DeviceGone), t + s(21));
    assert_eq!(r.health(), Health::AuthFailed);
    assert_eq!(r.last_error(), Some("the keyboard is no longer known to BlueZ"));
    // Plus appairé : une reprise ne tente rien.
    r.on_resume(false, t + s(30));
    assert_eq!(r.health(), Health::AuthFailed);
    assert_eq!(r.next_deadline(), None);

    let mut r = lost(t);
    let a = t + LOSS_GRACE;
    r.poll(a);
    r.on_connect_result(Err(ConnectError::AdapterOff), a);
    assert_eq!(r.next_deadline(), None);
    assert!(r.poll(a + s(1000)).is_empty());
    // L'adaptateur revient : nouvelle tentative après RESUME_GRACE, jamais à moins de 20 s de la précédente.
    r.on_adapter(true, a + s(1));
    assert_eq!(r.next_deadline(), Some(a + MIN_SPACING));
    r.on_adapter(true, a + s(100));
    assert_eq!(r.next_deadline(), Some(a + s(100) + RESUME_GRACE));
}

#[test]
fn adapter_events_in_other_states() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_adapter(true, t + s(5));
    assert_eq!(r.next_deadline(), None, "connecté : rien à planifier");
    r.on_adapter(false, t + s(6));
    r.on_disconnected(DisconnectReason::Timeout, t + s(7));
    assert_eq!(r.next_deadline(), None, "adaptateur éteint : aucune tentative");
    assert!(r.poll(t + s(100)).is_empty());
    r.on_adapter(true, t + s(200));
    assert_eq!(r.next_deadline(), Some(t + s(200) + RESUME_GRACE));
    assert_eq!(r.poll(t + s(204)), vec![Action::Connect]);
}

#[test]
fn spacing_never_closer_than_20_s() {
    let t = Instant::now();
    let mut r = lost(t);
    let a = t + LOSS_GRACE;
    assert_eq!(r.poll(a), vec![Action::Connect]);
    r.on_connect_result(Ok(()), a + s(1)); // libère le vol, fixe a+21
    r.on_adapter(true, a + s(2)); // brut a+6 < a+20 -> a+20
    assert_eq!(r.next_deadline(), Some(a + MIN_SPACING));
    r.on_adapter(true, a + s(16)); // brut a+20 -> a+20
    assert_eq!(r.next_deadline(), Some(a + MIN_SPACING));
    r.on_adapter(true, a + s(17)); // brut a+21 > a+20 -> a+21
    assert_eq!(r.next_deadline(), Some(a + s(21)));
    r.on_adapter(true, a + s(60));
    assert_eq!(r.next_deadline(), Some(a + s(64)));
}

#[test]
fn request_now_honours_spacing_and_state() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Remote, t); // prochaine tentative t+300
    r.request_now(t + s(10));
    assert_eq!(r.next_deadline(), Some(t + s(10)));
    // Une demande plus tardive ne repousse pas une échéance plus proche.
    r.request_now(t + s(100));
    assert_eq!(r.next_deadline(), Some(t + s(10)));
    assert_eq!(r.poll(t + s(10)), vec![Action::Connect]);
    r.on_connect_result(Ok(()), t + s(11)); // a = t+10, prochaine t+31
    r.request_now(t + s(12)); // brut t+12 < a+20 -> t+30 ; min(t+31, t+30)
    assert_eq!(r.next_deadline(), Some(t + s(30)));
    // Pendant une tentative : sans effet.
    assert_eq!(r.poll(t + s(30)), vec![Action::Connect]);
    r.request_now(t + s(31));
    assert_eq!(r.next_deadline(), None);
    // Connecté : sans effet.
    let mut c = Recovery::new();
    c.start(true, true, t);
    c.request_now(t);
    assert_eq!(c.next_deadline(), None);
    assert!(c.poll(t + s(1000)).is_empty());
    // Suspendu : sans effet.
    let mut z = lost(t);
    z.on_sleep(t + s(1));
    z.request_now(t + s(2));
    assert_eq!(z.next_deadline(), None);
    assert!(z.poll(t + s(1000)).is_empty());
}

#[test]
fn unreachable_thresholds() {
    let t = Instant::now();
    // 3 échecs mais 599 s : encore dormant.
    let mut r = lost(t);
    for _ in 0..3 {
        r.on_connect_result(Err(ConnectError::NoAnswer), t + UNREACHABLE_AFTER - s(1));
    }
    assert_eq!(r.health(), Health::Dormant);
    // 600 s mais 2 échecs.
    let mut r = lost(t);
    for _ in 0..2 {
        r.on_connect_result(Err(ConnectError::NoAnswer), t + UNREACHABLE_AFTER);
    }
    assert_eq!(r.health(), Health::Dormant);
    // 600 s et 3 échecs : injoignable, avec un avis.
    r.on_connect_result(Err(ConnectError::NoAnswer), t + UNREACHABLE_AFTER);
    assert_eq!(r.health(), Health::Unreachable);
    assert_eq!(r.poll(t + UNREACHABLE_AFTER), vec![Action::Notify(Notice::Unreachable { since: t })]);
    // Pas de second avis tant que l'épisode dure.
    r.on_connect_result(Err(ConnectError::NoAnswer), t + UNREACHABLE_AFTER + s(5));
    assert!(r.poll(t + s(3000)).iter().all(|a| !matches!(a, Action::Notify(_))));
    // Retour : avis « Recovered », une seule fois.
    r.on_connected(t + s(4000));
    assert_eq!(r.health(), Health::Connected);
    assert_eq!((r.attempts(), r.failures(), r.since(), r.last_error()), (0, 0, None, None));
    assert_eq!(r.poll(t + s(4000)), vec![Action::Notify(Notice::Recovered)]);
    assert!(r.poll(t + s(4001)).is_empty());
}

#[test]
fn quiet_episode_never_becomes_unreachable() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Local, t);
    for _ in 0..5 {
        r.on_connect_result(Err(ConnectError::NoAnswer), t + s(5000));
    }
    assert_eq!(r.health(), Health::Dormant);
    assert!(r.poll(t + s(5000)).iter().all(|a| !matches!(a, Action::Notify(_))));
}

#[test]
fn unreachable_notice_not_repeated_after_repair_notice() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.on_bond_lost(t);
    assert_eq!(r.health(), Health::AuthFailed);
    assert_eq!(
        r.poll(t),
        vec![Action::Notify(Notice::RepairNeeded { why: "the pairing was removed".into() })]
    );
    r.on_bond_lost(t + s(1)); // déjà refusé : pas de second avis
    assert!(r.poll(t + s(1)).is_empty());
    // Réappairé puis perdu : l'avis « injoignable » n'est pas redoublé (déjà notifié).
    r.start(false, true, t + s(2));
    for _ in 0..3 {
        r.on_connect_result(Err(ConnectError::NoAnswer), t + s(2) + UNREACHABLE_AFTER);
    }
    assert_eq!(r.health(), Health::Unreachable);
    assert!(r.poll(t + s(2) + UNREACHABLE_AFTER).iter().all(|a| !matches!(a, Action::Notify(_))));
}

#[test]
fn disconnect_reasons_open_the_right_episode() {
    let t = Instant::now();
    // Timeout / Unknown : épisode « perdu », première tentative après LOSS_GRACE.
    for reason in [DisconnectReason::Timeout, DisconnectReason::Unknown] {
        let mut r = Recovery::new();
        r.start(true, true, t);
        r.on_disconnected(reason, t + s(1));
        assert_eq!(r.next_deadline(), Some(t + s(1) + LOSS_GRACE));
        assert_eq!(r.last_reason(), Some(reason));
        assert_eq!(r.since(), Some(t + s(1)));
    }
    // Remote / Local : épisode calme, DORMANT_GRACE.
    for reason in [DisconnectReason::Remote, DisconnectReason::Local] {
        let mut r = Recovery::new();
        r.start(true, true, t);
        r.on_disconnected(reason, t);
        assert_eq!(r.next_deadline(), Some(t + DORMANT_GRACE));
    }
    // Depuis « inconnu » (aucun start) : épisode aussi.
    let mut r = Recovery::new();
    r.on_disconnected(DisconnectReason::Timeout, t);
    assert_eq!(r.health(), Health::Dormant);
    let mut r = Recovery::new();
    r.on_disconnected(DisconnectReason::Remote, t);
    assert_eq!(r.next_deadline(), Some(t + DORMANT_GRACE));
    // Déjà dormant : une nouvelle perte ne redémarre pas l'épisode.
    let mut r = lost(t);
    r.on_disconnected(DisconnectReason::Timeout, t + s(5));
    assert_eq!(r.since(), Some(t));
    assert_eq!(r.next_deadline(), Some(t + LOSS_GRACE));
    r.on_disconnected(DisconnectReason::Remote, t + s(6));
    assert_eq!(r.since(), Some(t));
    // Authentification : réparation demandée.
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Authentication, t);
    assert_eq!(r.health(), Health::AuthFailed);
    assert_eq!(r.last_error(), Some("the keyboard refused the stored pairing"));
    // Suspension demandée par l'hôte.
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Suspend, t + s(3));
    assert_eq!(r.health(), Health::Suspended);
    assert_eq!(r.since(), Some(t + s(3)));
    assert_eq!(r.next_deadline(), None);
    assert!(r.poll(t + s(9999)).is_empty());
}

#[test]
fn known_reason_refines_an_unknown_one() {
    let t = Instant::now();
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Unknown, t); // Connected=false d'abord
    assert_eq!(r.next_deadline(), Some(t + LOSS_GRACE));
    r.on_disconnected(DisconnectReason::Remote, t + s(1)); // puis le signal avec la raison
    assert_eq!(r.next_deadline(), Some(t + s(1) + DORMANT_GRACE));
    assert_eq!(r.last_reason(), Some(DisconnectReason::Remote));
    // Une raison inconnue ne raffine rien.
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Unknown, t);
    r.on_disconnected(DisconnectReason::Unknown, t + s(1));
    assert_eq!(r.next_deadline(), Some(t + LOSS_GRACE));
    // Après une tentative, plus de raffinement.
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Unknown, t);
    assert_eq!(r.poll(t + LOSS_GRACE), vec![Action::Connect]);
    r.on_connect_result(Err(ConnectError::NoAnswer), t + LOSS_GRACE);
    let d = r.next_deadline();
    r.on_disconnected(DisconnectReason::Remote, t + s(30));
    assert_eq!(r.next_deadline(), d);
    // Une raison précédente connue (non Unknown) ne raffine pas non plus.
    let mut r = Recovery::new();
    r.start(true, true, t);
    r.on_disconnected(DisconnectReason::Timeout, t);
    r.on_disconnected(DisconnectReason::Remote, t + s(1));
    assert_eq!(r.next_deadline(), Some(t + LOSS_GRACE));
}

#[test]
fn sleep_and_resume() {
    let t = Instant::now();
    let mut r = lost(t);
    r.on_sleep(t + s(2));
    assert_eq!(r.health(), Health::Suspended);
    assert_eq!(r.since(), Some(t), "l'épisode en cours est conservé");
    assert_eq!(r.next_deadline(), None);
    assert!(r.poll(t + s(5000)).is_empty());
    r.on_resume(false, t + s(100));
    assert_eq!(r.health(), Health::Dormant);
    assert_eq!(r.since(), Some(t + s(100)));
    assert_eq!(r.next_deadline(), Some(t + s(100) + RESUME_GRACE));
    // Reprise avec lien présent.
    let mut r = lost(t);
    r.on_sleep(t + s(2));
    r.on_resume(true, t + s(50));
    assert_eq!(r.health(), Health::Connected);
    assert_eq!(r.since(), None);
    // Sommeil depuis « inconnu » : l'épisode démarre à l'endormissement.
    let mut r = Recovery::new();
    r.on_sleep(t + s(7));
    assert_eq!((r.health(), r.since()), (Health::Suspended, Some(t + s(7))));
    // Un clavier refusé reste refusé, dans les deux sens.
    let mut r = Recovery::new();
    r.on_bond_lost(t);
    r.on_sleep(t + s(1));
    assert_eq!(r.health(), Health::AuthFailed);
    r.on_resume(true, t + s(2));
    assert_eq!(r.health(), Health::AuthFailed);
    r.on_resume(false, t + s(3));
    assert_eq!(r.health(), Health::AuthFailed);
}

#[test]
fn sleep_cancels_an_inflight_attempt() {
    let t = Instant::now();
    let mut r = lost(t);
    assert_eq!(r.poll(t + LOSS_GRACE), vec![Action::Connect]);
    r.on_sleep(t + s(21));
    r.on_resume(false, t + s(60));
    assert_eq!(r.poll(t + s(64)), vec![Action::Connect], "la tentative annulée ne bloque pas la suivante");
}

#[test]
fn connected_resets_the_episode() {
    let t = Instant::now();
    let mut r = lost(t);
    r.poll(t + LOSS_GRACE);
    r.on_connect_result(Err(ConnectError::NoAnswer), t + s(21));
    r.on_connected(t + s(30));
    assert_eq!((r.health(), r.attempts(), r.failures(), r.since(), r.last_error()), (Health::Connected, 0, 0, None, None));
    assert_eq!(r.next_deadline(), None);
    assert!(r.poll(t + s(10_000)).is_empty());
    // Un nouvel épisode repart du début du calendrier.
    r.on_disconnected(DisconnectReason::Timeout, t + s(100));
    assert_eq!(r.next_deadline(), Some(t + s(100) + LOSS_GRACE));
}
