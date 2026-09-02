#![forbid(unsafe_code)]

//! Corral's domain semantics and invariants: Session identity, bindings,
//! assurance, and evidence.
//!
//! This crate performs no IO and owns no wire vocabulary. Types that appear on
//! the wire live in `corral-protocol`, and surfaces depend on that crate rather
//! than on this one — a type crossing the wire does not move its business
//! semantics out of the domain (`ARCHITECTURE.md` §10).
//!
//! It also owns no durable encoding. `corral-state` decides how a domain fact
//! is written to disk, which is why nothing here derives a serialization: a
//! domain type must be free to change shape without silently changing what a
//! stored fact means.

mod assurance;
mod attention;
mod attention_state;
mod binding;
mod command;
mod entitlement;
mod evidence;
mod external_name;
mod id;
mod integration;
mod run;
mod session;

pub use assurance::Assurance;
pub use attention::{
    AllowedActions, AttentionAction, AttentionItem, AttentionReason, NeedsInputAction,
    NeedsInputContext, NeedsInputRequest,
};
pub use attention_state::{AttentionState, LastKnown, MainState, NotSemantic, SemanticState};
pub use binding::{
    Binding, BindingKey, BindingKind, ControlEligibility, IdentityStatus, NativeResumeEligibility,
    Provenance, ReservedNamespaceMisuse,
};
pub use command::{
    Command, CommandFingerprint, CommandFingerprintBuilder, CommandId, CommandKind, CommandOutcome,
    CommandReceipt, MalformedCommandId,
};
pub use entitlement::{Channel, Claim, Entitlement, Sealing};
pub use evidence::{Evidence, EvidenceSource};
pub use external_name::{ExternalId, MalformedExternalName, NameRefusal, ProviderId, ToolName};
pub use id::{
    AttentionItemId, BindingId, CorralSessionId, MalformedId, NeedsInputRequestId, NodeId, RunId,
};
pub use integration::{
    ConfigTarget, IntegrationIntent, RepairAuthority, RepairBudget, RepairFingerprint,
    RepairableDrift,
};
pub use run::{ExitCause, OccurrenceTime, Run, RunEnd, RunOrdinal};
pub use session::{LineageRefused, Session, SessionLineage};
