//! The daemon's two clocks, and which questions each one answers.
//!
//! Durations Corral acts on — a claim's freshness, a horizon, the echo window
//! after a keystroke — are differences between two readings of a clock that
//! only moves forward (ADR 0015 D5). Wall time answers a different question:
//! what to call a moment when a person or a journal record is told about it.
//! Measuring an age on the wall clock is what lets an NTP step make a fresh
//! claim stale, or revive one that had rotted, without any new evidence.

use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime};

/// Fixed when the daemon first asks what time it is.
static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);

/// A monotonic instant, held as the time since this daemon's origin.
///
/// Comparable and subtractable across the daemon, publishable through an
/// atomic, and meaningless outside this process — which is the point: it is
/// never a date and never goes on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Monotonic(Duration);

impl Monotonic {
    #[must_use]
    pub fn now() -> Self {
        Self(ORIGIN.elapsed())
    }

    /// How long after `earlier` this instant is, or `None` when it is not
    /// after it at all.
    #[must_use]
    pub fn since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }

    #[must_use]
    pub fn after(self, elapsed: Duration) -> Self {
        Self(self.0.saturating_add(elapsed))
    }

    /// Milliseconds since the origin, for publication through an atomic.
    #[must_use]
    pub fn as_millis(self) -> u64 {
        u64::try_from(self.0.as_millis()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn from_millis(millis: u64) -> Self {
        Self(Duration::from_millis(millis))
    }

    /// The same instant, for a caller that already holds one the platform
    /// gave it. Before the origin is not representable and reads as the
    /// origin, which is the earliest this daemon can mean.
    #[must_use]
    pub fn of(instant: Instant) -> Self {
        Self(instant.saturating_duration_since(*ORIGIN))
    }
}

/// One reading of both clocks: the instant ages are measured against, and the
/// wall time that same moment is called by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    pub mono: Monotonic,
    pub wall: SystemTime,
}

impl Reading {
    #[must_use]
    pub fn now() -> Self {
        Self {
            mono: Monotonic::now(),
            wall: SystemTime::now(),
        }
    }

    /// The same reading, moved back to an earlier monotonic instant. The wall
    /// time is measured back from this reading rather than read again, so an
    /// age and the date it implies always agree.
    #[must_use]
    pub fn at(self, mono: Monotonic) -> Self {
        let wall = self
            .mono
            .since(mono)
            .and_then(|age| self.wall.checked_sub(age))
            .unwrap_or(self.wall);
        Self { mono, wall }
    }
}

#[cfg(test)]
#[path = "clock_tests.rs"]
mod tests;
