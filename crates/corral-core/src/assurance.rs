/// How sure Corral is of a binding's association.
///
/// Discrete levels, never a floating confidence score (`ARCHITECTURE.md` §1).
/// Deliberately not ordered: a comparison operator would invite confidence
/// arithmetic — taking the maximum of two levels, averaging evidence — and the
/// domain asks a level exactly the two questions below.
///
/// Assurance lives on the binding and nowhere else. A Run carries none, and no
/// second carrier exists (ADR 0002, Q8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Assurance {
    /// `corrald` spawned and owns the runtime; identity holds by construction.
    Deterministic,
    /// Live provider-native evidence proves the binding.
    Attested,
    /// The user explicitly linked it.
    Manual,
    /// cwd / time / process / history correlation only.
    Heuristic,
}

impl Assurance {
    /// Whether a binding at this level may drive cross-facet control
    /// (AGENTS.md §Core model).
    #[must_use]
    pub fn permits_control(self) -> bool {
        match self {
            Self::Deterministic | Self::Attested | Self::Manual => true,
            Self::Heuristic => false,
        }
    }

    /// Whether a fact resting on this level may be written to the durable
    /// semantic event log.
    ///
    /// Durability follows fact assurance, not object existence (ADR 0002 D6):
    /// writing `RunStarted` into a Session's stream durably asserts that the
    /// Run belongs to it, and under a Heuristic binding that assertion is a
    /// guess.
    ///
    /// A second predicate rather than a call to `permits_control`: these are
    /// two separate laws that select the same levels today, and collapsing
    /// them would let a change to one silently move the other.
    #[must_use]
    pub fn may_assert_durable_fact(self) -> bool {
        match self {
            Self::Deterministic | Self::Attested | Self::Manual => true,
            Self::Heuristic => false,
        }
    }
}

#[cfg(test)]
#[path = "assurance_tests.rs"]
mod tests;
