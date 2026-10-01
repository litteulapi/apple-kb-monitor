//! Binaire de test dédié : l'état global de `read_policy` (dernier appui de
//! touche) est vierge au démarrage, ce qu'un binaire partagé ne garantit pas.

use akm_core::read_policy::{last_input_age, note_input};
use std::time::{Duration, Instant};

#[test]
fn no_input_then_input_age_follows_the_clock() {
    // Aucun appui noté : pas d'âge (et donc clavier « inactif »).
    assert_eq!(last_input_age(Instant::now()), None);
    note_input();
    let age = last_input_age(Instant::now()).expect("un appui vient d'être noté");
    assert!(age < Duration::from_secs(1), "âge {age:?}");
    // L'âge croît avec l'horloge, à la milliseconde près.
    let later = last_input_age(Instant::now() + Duration::from_secs(10)).unwrap();
    assert!(later >= Duration::from_secs(10) && later < Duration::from_secs(11), "âge {later:?}");
    // Un `now` antérieur à l'appui ne donne jamais un âge négatif / énorme.
    let earlier = last_input_age(Instant::now() - Duration::from_millis(1));
    assert!(earlier.unwrap() < Duration::from_millis(5));
}
