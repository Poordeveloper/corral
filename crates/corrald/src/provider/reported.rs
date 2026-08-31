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
use std::time::Instant;

use corral_core::{CorralSessionId, ExternalId, RunId};

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
    /// The Run this Session's identity was last observed in.
    ///
    /// What makes a re-observation worth recording durably. Codex reports no
    /// session start, so "a fresh Run of the same conversation" cannot be read
    /// off an event name for every provider — but it can be read off the Run,
    /// which is the fact the confirmation was always about (ADR 0004 D7,
    /// ADR 0009 D3). Held here so the check costs no store hop: without it,
    /// every turn of every launch would take the blocking pool and the store
    /// lock to be told the identity has not changed.
    observed_in: Option<RunId>,
    /// How far this Session's identity question is closed.
    ///
    /// Held here so the ingestion step can answer "this changes nothing"
    /// without a store hop: a session whose question is closed goes on firing
    /// hooks for its whole life with an identity claim of `None`, so without
    /// this it is exactly the session that pays the trip on every prompt, to
    /// be turned away by the same verdict each time.
    closure: IdentityClosure,
    /// When that fact reached this daemon, on the monotonic clock.
    ///
    /// Supersession is decided on this and not on `latest.observed_at`. The
    /// wall clock a fact is stamped with is what a surface renders an age
    /// from, and it can step backwards — NTP, or a person setting it — after
    /// which an order taken from it would discard every later fact until the
    /// clock caught up, leaving a row asserting a turn that has since ended.
    arrived: Option<Instant>,
}

/// How closed a Session's identity question is, and against what.
///
/// A contest closes it whole: Corral withdrew the claim as unsafe, and no
/// later report of any id reopens it (ADR 0004 D8). An identity another
/// Session holds closes exactly that id — the agent can still mint a fresh
/// one (`/clear` does), and a report of one deserves the store's answer, not
/// this daemon's memory of a different refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
enum IdentityClosure {
    Open,
    Contested,
    ClaimedElsewhere(ExternalId),
}

impl ReportedSession {
    fn new(provider: KnownProvider) -> Self {
        Self {
            provider,
            external_id: None,
            observed_in: None,
            closure: IdentityClosure::Open,
            latest: None,
            arrived: None,
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

    /// Publish the provider identity Corral now stands behind, and the Run it
    /// was observed in.
    ///
    /// One call rather than two, because the two are one event: an identity
    /// Corral stands behind was observed somewhere, and a Run that had
    /// observed it without standing behind it would be a claim with no
    /// evidence and evidence with no claim.
    pub fn identified(
        &mut self,
        session: CorralSessionId,
        provider: KnownProvider,
        run: RunId,
        external_id: ExternalId,
    ) {
        let held = self
            .sessions
            .entry(session)
            .or_insert_with(|| ReportedSession::new(provider));
        held.external_id = Some(external_id);
        held.observed_in = Some(run);
    }

    /// Whether this Run has already observed exactly this identity.
    ///
    /// `false` for a Session this daemon holds nothing about, which is the
    /// honest answer: live state is lost on restart, and the store is the
    /// authority every path falls back to.
    pub fn identity_observed_in(
        &self,
        session: CorralSessionId,
        run: RunId,
        reported: &ExternalId,
    ) -> bool {
        self.sessions.get(&session).is_some_and(|held| {
            held.observed_in == Some(run) && held.external_id.as_ref() == Some(reported)
        })
    }

    /// Record that this Session's identity claim was contested, and withdraw
    /// exactly that claim.
    ///
    /// The provider and the reported facts stay, and the conflicting id is
    /// never promoted into a replacement (ADR 0004 D8). One call rather than
    /// two, because the withdrawal only ever happens for this reason: a
    /// surface that could see the claim gone without the cause would have to
    /// guess at it.
    pub fn contested(&mut self, session: CorralSessionId) {
        if let Some(reported) = self.sessions.get_mut(&session) {
            reported.external_id = None;
            reported.observed_in = None;
            reported.closure = IdentityClosure::Contested;
        }
    }

    /// Record that the identity this Session reported belongs to another one.
    ///
    /// No claim to withdraw — this Session never had one — and none to make:
    /// binding uniqueness is what stops one external identity resolving to two
    /// Sessions, and nothing in this phase unbinds. What is recorded is that
    /// asking about *this id* again will not answer differently. Only this id:
    /// the refusal says nothing about an identity the agent has not minted
    /// yet, and a contest already standing is not weakened to it.
    pub fn identity_claimed_elsewhere(&mut self, session: CorralSessionId, refused: ExternalId) {
        if let Some(reported) = self.sessions.get_mut(&session)
            && !matches!(reported.closure, IdentityClosure::Contested)
        {
            reported.closure = IdentityClosure::ClaimedElsewhere(refused);
        }
    }

    /// Whether a report of exactly this identity can change anything.
    ///
    /// `false` for a Session this daemon holds nothing about, which is the
    /// honest answer: live state is lost on restart, and the store is the
    /// authority every path falls back to.
    pub fn identity_closed(&self, session: CorralSessionId, reported: &ExternalId) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|held| match &held.closure {
                IdentityClosure::Open => false,
                IdentityClosure::Contested => true,
                IdentityClosure::ClaimedElsewhere(refused) => refused == reported,
            })
    }

    /// Record the latest fact an agent reported about itself.
    ///
    /// The latest by arrival at this daemon. Each hook is delivered by its own
    /// process over its own connection, so two events fired back to back can
    /// be accepted in either order under ordinary scheduling, and replacing
    /// unconditionally would let a row go backwards — `latest` would stop
    /// meaning what it says.
    ///
    /// `arrived` is the monotonic instant the endpoint took, not the wall
    /// clock it stamped the fact with: this is a live view of one process's
    /// own observations, so the order it saw them in is knowable without
    /// trusting a clock that can be stepped.
    pub fn reported(
        &mut self,
        session: CorralSessionId,
        provider: KnownProvider,
        fact: AgentFact,
        arrived: Instant,
    ) {
        let held = self
            .sessions
            .entry(session)
            .or_insert_with(|| ReportedSession::new(provider));
        let supersedes = held.arrived.is_none_or(|latest| latest <= arrived);
        if supersedes {
            held.latest = Some(fact);
            held.arrived = Some(arrived);
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
