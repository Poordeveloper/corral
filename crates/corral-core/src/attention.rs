use crate::evidence::Evidence;
use crate::external_name::{ProviderId, ToolName};
use crate::id::{CorralSessionId, NeedsInputRequestId};

/// A structured reason a Session needs the user.
///
/// Structured from day one because an attention boolean cannot be upgraded
/// into an answerable request compatibly (`ARCHITECTURE.md` §2). This is the
/// shared domain meaning only: deriving attention, ranking it, notifying on
/// it, and rendering it belong to the daemon-side Attention Engine and the
/// surfaces (PR8), and none of it is decided here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionItem {
    session: CorralSessionId,
    reason: AttentionReason,
    evidence: Evidence,
    action: Option<AttentionAction>,
}

impl AttentionItem {
    #[must_use]
    pub fn new(
        session: CorralSessionId,
        reason: AttentionReason,
        evidence: Evidence,
        action: Option<AttentionAction>,
    ) -> Self {
        Self {
            session,
            reason,
            evidence,
            action,
        }
    }

    #[must_use]
    pub fn session(&self) -> CorralSessionId {
        self.session
    }

    #[must_use]
    pub fn reason(&self) -> AttentionReason {
        self.reason
    }

    /// The observation this item rests on. Its source and `observed_at` are
    /// what a later phase judges freshness and authority by; an item does not
    /// judge itself.
    #[must_use]
    pub fn evidence(&self) -> Evidence {
        self.evidence
    }

    /// The specific thing that would resolve this item, when Corral can name
    /// one. `None` means Corral cannot name it — never that nothing would.
    #[must_use]
    pub fn action(&self) -> Option<&AttentionAction> {
        self.action.as_ref()
    }
}

/// Why a Session is asking for the user.
///
/// Only actionable reasons: a Session that is working, or whose status Corral
/// cannot claim, is not asking for anything. The user-visible state model
/// (`PRODUCT.md` §4) is derived from evidence by PR8 and is not this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttentionReason {
    /// The agent is blocked on user input, approval, or an answer.
    NeedsInput,
    /// The turn finished and is waiting to be looked at.
    TurnComplete,
    /// The runtime ended.
    RuntimeEnded,
}

/// What would resolve an attention item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionAction {
    /// Answer a specific blocked interaction.
    Answer(NeedsInputRequestId),
}

/// A specific blocked interaction awaiting an answer.
///
/// Reserved now and answered by attaching the terminal in M1; structured
/// approval UI is M2. What matters at this phase is that the entity exists and
/// is addressable, so that a later surface can answer *this* request rather
/// than a session-wide flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedsInputRequest {
    id: NeedsInputRequestId,
    session: CorralSessionId,
    context: NeedsInputContext,
    allowed_actions: AllowedActions,
}

impl NeedsInputRequest {
    #[must_use]
    pub fn new(
        id: NeedsInputRequestId,
        session: CorralSessionId,
        context: NeedsInputContext,
        allowed_actions: AllowedActions,
    ) -> Self {
        Self {
            id,
            session,
            context,
            allowed_actions,
        }
    }

    #[must_use]
    pub fn id(&self) -> NeedsInputRequestId {
        self.id
    }

    #[must_use]
    pub fn session(&self) -> CorralSessionId {
        self.session
    }

    #[must_use]
    pub fn context(&self) -> &NeedsInputContext {
        &self.context
    }

    #[must_use]
    pub fn allowed_actions(&self) -> &AllowedActions {
        &self.allowed_actions
    }
}

/// What the provider said it is blocked on.
///
/// Provider vocabulary, kept verbatim: Corral does not normalize tool names
/// into a Corral taxonomy, because a taxonomy that guesses wrong presents the
/// user with an operation nobody is performing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedsInputContext {
    provider: ProviderId,
    tool: Option<ToolName>,
}

impl NeedsInputContext {
    #[must_use]
    pub fn new(provider: ProviderId, tool: Option<ToolName>) -> Self {
        Self { provider, tool }
    }

    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// The tool the provider named, when it named one.
    #[must_use]
    pub fn tool(&self) -> Option<&ToolName> {
        self.tool.as_ref()
    }
}

/// Which answers a request will accept.
///
/// An enum rather than an empty list because absence must never be read as a
/// known negative (AGENTS.md §Protocol): "Corral does not know which answers
/// this request takes" and "this request takes no answers" are different
/// facts, and only the second may disable a surface's answer path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowedActions {
    Unknown,
    Exactly(Vec<NeedsInputAction>),
}

/// One answer a blocked interaction will accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NeedsInputAction {
    Allow,
    Deny,
    Answer,
}

#[cfg(test)]
#[path = "attention_tests.rs"]
mod tests;
