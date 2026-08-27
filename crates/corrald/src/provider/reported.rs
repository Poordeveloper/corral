//! What providers have reported about the sessions this daemon runs.
//!
//! Live evidence, and live is the whole lifetime: a daemon restart loses it,
//! the secondary facts vanish from every row, and the honest answer returns to
//! bare runtime truth (ADR 0004 D7). Nothing here is persisted — a raw hook
//! event is never a durable fact (`ARCHITECTURE.md` §5).
//!
//! The one thing it must not do is disagree with the log. It cannot: every
//! change here is applied from the same ingestion step that decided what the
//! log takes, so a claim withdrawn durably is withdrawn here in the same
//! breath.

use std::collections::HashMap;

use corral_core::{CorralSessionId, ExternalId};

use super::{AgentFact, KnownProvider};

/// The provider facts a surface may currently be told about one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportedSession {
    /// Which agent product this session runs.
    ///
    /// Known from the managed launch itself, before any hook fires, and
    /// retained through an identity contest: the product is not what became
    /// ambiguous (ADR 0004 D8, R2 Q3).
    pub provider: KnownProvider,
    /// The provider identity Corral currently stands behind.
    ///
    /// `None` before one is learned, and `None` again after a contest —
    /// meaning not currently assertable, never that no id ever existed. The
    /// durable log keeps the original id, the conflicting report, and their
    /// evidence as provenance; this field is a current claim, and one field
    /// never means both.
    pub external_id: Option<ExternalId>,
    /// The latest still-relevant fact the agent reported, or `None` when it
    /// has reported nothing.
    ///
    /// Superseded rather than accumulated: a newer fact retires the older one,
    /// so an `awaiting_input` is not still on screen after a turn started.
    pub latest: Option<AgentFact>,
}

impl ReportedSession {
    fn new(provider: KnownProvider) -> Self {
        Self {
            provider,
            external_id: None,
            latest: None,
        }
    }
}

/// Everything providers have reported, by Session.
#[derive(Default)]
pub struct ReportedSessions {
    sessions: HashMap<CorralSessionId, ReportedSession>,
}

impl ReportedSessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a Session runs this provider, before it has reported
    /// anything.
    ///
    /// Called at launch so the *first* fact to arrive is attributed to the
    /// agent Corral started rather than to whatever a payload claims, and so
    /// `session.list` carries the provider from the moment the Session exists.
    /// The terminal surfaces draw nothing from it on its own — a row already
    /// names the program it launched — but the field is a fact and the title
    /// is a label, and a surface that wants the fact should not have to wait
    /// for a hook to get it.
    ///
    /// Idempotent across a resume: the same Session keeps whatever it has
    /// already learned.
    pub fn launched(&mut self, session: CorralSessionId, provider: KnownProvider) {
        self.sessions
            .entry(session)
            .or_insert_with(|| ReportedSession::new(provider));
    }

    /// Publish the provider identity Corral now stands behind.
    pub fn identified(
        &mut self,
        session: CorralSessionId,
        provider: KnownProvider,
        external_id: ExternalId,
    ) {
        self.sessions
            .entry(session)
            .or_insert_with(|| ReportedSession::new(provider))
            .external_id = Some(external_id);
    }

    /// Withdraw the current identity claim, leaving everything still known.
    ///
    /// Withdraw exactly the claim that became unsafe: the provider and the
    /// reported facts stay, and the conflicting id is never promoted into a
    /// replacement (ADR 0004 D8).
    pub fn withdraw_identity(&mut self, session: CorralSessionId) {
        if let Some(reported) = self.sessions.get_mut(&session) {
            reported.external_id = None;
        }
    }

    /// Record the latest fact an agent reported about itself.
    ///
    /// The latest by observation, not by arrival. Each hook is delivered by its
    /// own process over its own connection, and the endpoint stamps each on
    /// arrival — so two events fired back to back can be accepted in either
    /// order under ordinary scheduling. Replacing unconditionally would let a
    /// row go backwards, and `latest` would stop meaning what it says.
    pub fn reported(&mut self, session: CorralSessionId, provider: KnownProvider, fact: AgentFact) {
        let held = self
            .sessions
            .entry(session)
            .or_insert_with(|| ReportedSession::new(provider));
        let supersedes = held
            .latest
            .is_none_or(|latest| latest.observed_at <= fact.observed_at);
        if supersedes {
            held.latest = Some(fact);
        }
    }

    pub fn get(&self, session: CorralSessionId) -> Option<&ReportedSession> {
        self.sessions.get(&session)
    }

    /// Forget a Session that never came to exist.
    ///
    /// A launch that failed before its Session became a durable fact leaves an
    /// entry nothing can ever present. Dropping it is bookkeeping, not a
    /// withdrawal: there is no claim here to withdraw.
    pub fn forget(&mut self, session: CorralSessionId) {
        self.sessions.remove(&session);
    }
}

#[cfg(test)]
#[path = "reported_tests.rs"]
mod tests;
