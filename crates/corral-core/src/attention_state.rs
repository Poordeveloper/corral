use std::time::SystemTime;

/// The main status a person reads for a Session (`PRODUCT.md` §4).
///
/// Derived, never asserted by any single source: the attention engine in
/// `corrald` computes it from entitled evidence under ADR 0015, and clients
/// render it. Exited and Unknown are the two values no semantic source may
/// claim — Exited is execution truth, Unknown is the honest absence of a
/// fresh claim — which is what `SemanticState` exists to keep apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MainState {
    Working,
    NeedsYou,
    Ready,
    Unknown,
    Exited,
}

impl MainState {
    /// Whether this is a claim a semantic source can make at all.
    #[must_use]
    pub fn is_semantic(self) -> bool {
        SemanticState::try_from(self).is_ok()
    }
}

/// What a semantic evidence source may assert: one of the three states that
/// say what the agent is doing, never the two that say Corral cannot tell or
/// that the runtime is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticState {
    Working,
    NeedsYou,
    Ready,
}

impl From<SemanticState> for MainState {
    fn from(state: SemanticState) -> Self {
        match state {
            SemanticState::Working => Self::Working,
            SemanticState::NeedsYou => Self::NeedsYou,
            SemanticState::Ready => Self::Ready,
        }
    }
}

/// The main state was not a semantic claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotSemantic(pub MainState);

impl TryFrom<MainState> for SemanticState {
    type Error = NotSemantic;

    fn try_from(state: MainState) -> Result<Self, Self::Error> {
        match state {
            MainState::Working => Ok(Self::Working),
            MainState::NeedsYou => Ok(Self::NeedsYou),
            MainState::Ready => Ok(Self::Ready),
            MainState::Unknown | MainState::Exited => Err(NotSemantic(state)),
        }
    }
}

/// The last reliable fact about a Session, kept as secondary text once the
/// main state has rotted to Unknown — "Last known: Needed input 45m ago".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastKnown {
    state: MainState,
    at: SystemTime,
}

impl LastKnown {
    #[must_use]
    pub fn new(state: MainState, at: SystemTime) -> Self {
        Self { state, at }
    }

    #[must_use]
    pub fn state(&self) -> MainState {
        self.state
    }

    #[must_use]
    pub fn at(&self) -> SystemTime {
        self.at
    }
}

/// A Session's derived attention state: the main state, since when, and —
/// only beneath Unknown — what was last reliably known.
///
/// Two constructors rather than public fields so the pairing is unwritable:
/// a last-known fact beside Working would be a second claim beside the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttentionState {
    main: MainState,
    since: SystemTime,
    last_known: Option<LastKnown>,
}

impl AttentionState {
    /// A state the engine asserts — or Exited, which execution truth
    /// establishes — from a stated instant.
    #[must_use]
    pub fn asserted(main: MainState, since: SystemTime) -> Self {
        Self {
            main,
            since,
            last_known: None,
        }
    }

    /// No fresh claim, with what was last reliably known when there was one.
    #[must_use]
    pub fn unknown(since: SystemTime, last_known: Option<LastKnown>) -> Self {
        Self {
            main: MainState::Unknown,
            since,
            last_known,
        }
    }

    #[must_use]
    pub fn main(&self) -> MainState {
        self.main
    }

    #[must_use]
    pub fn since(&self) -> SystemTime {
        self.since
    }

    #[must_use]
    pub fn last_known(&self) -> Option<LastKnown> {
        self.last_known
    }
}

#[cfg(test)]
#[path = "attention_state_tests.rs"]
mod tests;
