use std::time::SystemTime;

use crate::id::{BindingId, CorralSessionId, RunId};

/// When a runtime fact actually happened, and how well Corral knows it.
///
/// Event sequence is the order Corral accepted a fact; occurrence time is when
/// the fact happened, and the two are independent — a `RunStarted` accepted
/// now may carry an occurrence twenty minutes old (ADR 0002 D6).
///
/// The variants exist so that a first-observed time cannot be handed to a
/// parameter named `started_at`. Only an authoritative occurrence time is
/// recorded durably; a first-observed time is live metadata, and persisting it
/// beside a start time is exactly the dressing-up D6 forbids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccurrenceTime {
    /// Independently supported by authoritative runtime evidence.
    Authoritative(SystemTime),
    /// The first moment Corral saw the fact. Says nothing about when it began.
    FirstObserved(SystemTime),
    /// Nothing supports a time at all. A first-class answer: an invented
    /// instant would read exactly like a measured one.
    Unknown,
}

impl OccurrenceTime {
    /// The instant, if and only if the runtime itself supports it.
    #[must_use]
    pub fn authoritative(self) -> Option<SystemTime> {
        match self {
            Self::Authoritative(at) => Some(at),
            Self::FirstObserved(_) | Self::Unknown => None,
        }
    }
}

/// How a Run stopped.
///
/// Platform detail — exit codes, signal numbers — is mapped into these by the
/// runtime owner and never leaks into the domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEnd {
    /// Observed to have exited, with the cause when determinable.
    Exited(ExitCause),
    /// `corrald` could not establish that the runtime exited. Never assumed
    /// exited (AGENTS.md §Runtime truth).
    Unverifiable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCause {
    Completed,
    Failed,
    Terminated,
    /// The exit was observed; its cause was not.
    Unknown,
}

/// One concrete runtime occurrence of a Session.
///
/// A Run records that a runtime existed. Its `RuntimeBinding` relates that
/// runtime to a Session and carries the assurance of that association. Run
/// existence alone never grants control eligibility (ADR 0002).
///
/// So this type deliberately exposes no assurance and no control question: the
/// `session` below is a structural reference to the current association, and
/// its trustworthiness is the referenced binding's, never the Run's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    id: RunId,
    session: CorralSessionId,
    runtime_binding: BindingId,
    ordinal: RunOrdinal,
    started: OccurrenceTime,
    ended: Option<(RunEnd, OccurrenceTime)>,
}

impl Run {
    #[must_use]
    pub fn started(
        id: RunId,
        session: CorralSessionId,
        runtime_binding: BindingId,
        ordinal: RunOrdinal,
        started: OccurrenceTime,
    ) -> Self {
        Self {
            id,
            session,
            runtime_binding,
            ordinal,
            started,
            ended: None,
        }
    }

    #[must_use]
    pub fn id(&self) -> RunId {
        self.id
    }

    /// The Session this Run is currently associated with, through
    /// `runtime_binding`. Never a control credential on its own.
    #[must_use]
    pub fn session(&self) -> CorralSessionId {
        self.session
    }

    /// The binding that carries the assurance of this association, and the
    /// only place control eligibility may be resolved from.
    #[must_use]
    pub fn runtime_binding(&self) -> BindingId {
        self.runtime_binding
    }

    #[must_use]
    pub fn ordinal(&self) -> RunOrdinal {
        self.ordinal
    }

    #[must_use]
    pub fn started_at(&self) -> OccurrenceTime {
        self.started
    }

    #[must_use]
    pub fn end(&self) -> Option<RunEnd> {
        self.ended.map(|(end, _)| end)
    }

    #[must_use]
    pub fn ended_at(&self) -> Option<OccurrenceTime> {
        self.ended.map(|(_, at)| at)
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        self.ended.is_none()
    }

    #[must_use]
    pub fn ended(mut self, end: RunEnd, at: OccurrenceTime) -> Self {
        self.ended = Some((end, at));
        self
    }
}

/// A Run's position within its Session, for display only.
///
/// Never an identity and never a reference: correcting a wrong binding
/// renumbers it, and a renumbered reference is a rewritten fact (ADR 0002 D1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RunOrdinal(u32);

impl RunOrdinal {
    /// The first Run of a Session.
    pub const FIRST: Self = Self(1);

    #[must_use]
    pub fn from_position(position: u32) -> Self {
        Self(position)
    }

    #[must_use]
    pub fn position(self) -> u32 {
        self.0
    }

    /// The ordinal after this one, saturating rather than wrapping: a Session
    /// with four billion Runs is a display problem, not a reason to reuse a
    /// number that already names another Run on screen.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
