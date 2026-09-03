use std::time::{Duration, SystemTime};

use super::*;

fn reading(mono_millis: u64, wall_secs: u64) -> Reading {
    Reading {
        mono: Monotonic::from_millis(mono_millis),
        wall: SystemTime::UNIX_EPOCH + Duration::from_secs(wall_secs),
    }
}

/// An earlier instant is named by measuring back from the reading that names
/// it, never by reading the wall clock again: the age shown to a person and
/// the date behind it then agree, whatever the wall clock did in between.
#[test]
fn an_earlier_instant_is_named_by_measuring_back() {
    let now = reading(10_000, 10_000);
    let earlier = now.at(Monotonic::from_millis(4_000));
    assert_eq!(earlier.mono, Monotonic::from_millis(4_000));
    assert_eq!(
        earlier.wall,
        SystemTime::UNIX_EPOCH + Duration::from_secs(9_994)
    );
}

/// An instant that is not earlier has no age, and is not silently turned into
/// one: `None` is what "this did not happen before now" has to mean, because
/// the alternative is an age of zero, which reads as "just now".
#[test]
fn an_instant_that_is_not_earlier_has_no_age() {
    assert_eq!(
        Monotonic::from_millis(1).since(Monotonic::from_millis(5)),
        None
    );
    assert_eq!(
        Monotonic::from_millis(5).since(Monotonic::from_millis(1)),
        Some(Duration::from_millis(4))
    );
}

/// Naming an instant the reading cannot reach back to falls forward to the
/// reading itself rather than inventing a time before the epoch.
#[test]
fn an_instant_later_than_the_reading_is_named_by_the_reading() {
    let now = reading(1_000, 5);
    assert_eq!(now.at(Monotonic::from_millis(4_000)).wall, now.wall);
}
