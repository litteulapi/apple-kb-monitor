//! Dedicated test binary: the global state of `read_policy` (last key press) is blank at
//! start, which a shared binary does not guarantee.

use akm_core::read_policy::{last_input_age, note_input};
use std::time::{Duration, Instant};

#[test]
fn no_input_then_input_age_follows_the_clock() {
    assert_eq!(last_input_age(Instant::now()), None);
    note_input();
    let age = last_input_age(Instant::now()).expect("a key press was just recorded");
    assert!(age < Duration::from_secs(1), "age {age:?}");
    let later = last_input_age(Instant::now() + Duration::from_secs(10)).unwrap();
    assert!(
        later >= Duration::from_secs(10) && later < Duration::from_secs(11),
        "age {later:?}"
    );
    // A `now` earlier than the key press never gives a negative / huge age.
    let earlier = last_input_age(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
    );
    assert!(earlier.unwrap() < Duration::from_millis(5));
}
