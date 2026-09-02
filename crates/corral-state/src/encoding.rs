//! How a domain value is written to disk.
//!
//! Every token here is durable vocabulary: once a store has been written with
//! one, changing it rewrites the meaning of recorded facts. They are spelled
//! out rather than derived from Rust names so that renaming a variant is a
//! compile error here instead of a silent reinterpretation of the log.

use std::time::{Duration, SystemTime};

use corral_core::{
    Assurance, BindingKind, CommandOutcome, ConfigTarget, EvidenceSource, ExitCause,
    IdentityStatus, IntegrationIntent, MalformedId, Provenance, RepairableDrift, RunEnd,
};

use crate::error::FatalState;

pub(crate) fn assurance_token(assurance: Assurance) -> &'static str {
    match assurance {
        Assurance::Deterministic => "deterministic",
        Assurance::Attested => "attested",
        Assurance::Manual => "manual",
        Assurance::Heuristic => "heuristic",
    }
}

pub(crate) fn assurance_from_token(token: &str) -> Result<Assurance, FatalState> {
    match token {
        "deterministic" => Ok(Assurance::Deterministic),
        "attested" => Ok(Assurance::Attested),
        "manual" => Ok(Assurance::Manual),
        "heuristic" => Ok(Assurance::Heuristic),
        other => Err(unreadable("an assurance level", other)),
    }
}

pub(crate) fn evidence_source_token(source: EvidenceSource) -> &'static str {
    match source {
        EvidenceSource::CorralConstructed => "corral-constructed",
        EvidenceSource::NodeRuntimeObservation => "node-runtime-observation",
        EvidenceSource::ProviderHook => "provider-hook",
        EvidenceSource::InBandSignal => "in-band-signal",
        // Never written by any accepted event — derived status is not durable
        // (ADR 0015 D8) — but the encoding is total, so a source that reaches
        // it by mistake is a readable token rather than a panic.
        EvidenceSource::PtyActivity => "pty-activity",
        EvidenceSource::ScreenDetection => "screen-detection",
        EvidenceSource::HistoryRecord => "history-record",
        EvidenceSource::Correlation => "correlation",
        EvidenceSource::UserAssertion => "user-assertion",
    }
}

pub(crate) fn evidence_source_from_token(token: &str) -> Result<EvidenceSource, FatalState> {
    match token {
        "corral-constructed" => Ok(EvidenceSource::CorralConstructed),
        "node-runtime-observation" => Ok(EvidenceSource::NodeRuntimeObservation),
        "provider-hook" => Ok(EvidenceSource::ProviderHook),
        "in-band-signal" => Ok(EvidenceSource::InBandSignal),
        "pty-activity" => Ok(EvidenceSource::PtyActivity),
        "screen-detection" => Ok(EvidenceSource::ScreenDetection),
        "history-record" => Ok(EvidenceSource::HistoryRecord),
        "correlation" => Ok(EvidenceSource::Correlation),
        "user-assertion" => Ok(EvidenceSource::UserAssertion),
        other => Err(unreadable("an evidence source", other)),
    }
}

/// Whether Corral still stands behind the identity a binding names.
///
/// A projection column rather than a payload field of `BindingAdded`: a
/// binding is always born `Confirmed`, and a log able to spell a
/// born-contested edge would let a fact exist that nothing can produce
/// (ADR 0004 D8).
pub(crate) fn identity_status_token(status: IdentityStatus) -> &'static str {
    match status {
        IdentityStatus::Confirmed => "confirmed",
        IdentityStatus::Contested => "contested",
    }
}

pub(crate) fn identity_status_from_token(token: &str) -> Result<IdentityStatus, FatalState> {
    match token {
        "confirmed" => Ok(IdentityStatus::Confirmed),
        "contested" => Ok(IdentityStatus::Contested),
        other => Err(unreadable("an identity status", other)),
    }
}

pub(crate) fn binding_kind_token(kind: BindingKind) -> &'static str {
    match kind {
        BindingKind::ProviderSession => "provider-session",
        BindingKind::Runtime => "runtime",
        BindingKind::Terminal => "terminal",
        BindingKind::History => "history",
    }
}

pub(crate) fn binding_kind_from_token(token: &str) -> Result<BindingKind, FatalState> {
    match token {
        "provider-session" => Ok(BindingKind::ProviderSession),
        "runtime" => Ok(BindingKind::Runtime),
        "terminal" => Ok(BindingKind::Terminal),
        "history" => Ok(BindingKind::History),
        other => Err(unreadable("a binding kind", other)),
    }
}

pub(crate) fn provenance_token(provenance: Provenance) -> &'static str {
    match provenance {
        Provenance::CorralCreated => "corral-created",
        Provenance::Discovered => "discovered",
        Provenance::UserLinked => "user-linked",
    }
}

pub(crate) fn provenance_from_token(token: &str) -> Result<Provenance, FatalState> {
    match token {
        "corral-created" => Ok(Provenance::CorralCreated),
        "discovered" => Ok(Provenance::Discovered),
        "user-linked" => Ok(Provenance::UserLinked),
        other => Err(unreadable("a provenance", other)),
    }
}

/// An unverifiable end and an exit whose cause was not determined are distinct
/// facts and stay distinct on disk: unreachable is never recorded as exited.
pub(crate) fn run_end_token(end: RunEnd) -> &'static str {
    match end {
        RunEnd::Exited(ExitCause::Completed) => "completed",
        RunEnd::Exited(ExitCause::Failed) => "failed",
        RunEnd::Exited(ExitCause::Terminated) => "terminated",
        RunEnd::Exited(ExitCause::Unknown) => "exited-cause-unknown",
        RunEnd::Unverifiable => "unverifiable",
    }
}

pub(crate) fn run_end_from_token(token: &str) -> Result<RunEnd, FatalState> {
    match token {
        "completed" => Ok(RunEnd::Exited(ExitCause::Completed)),
        "failed" => Ok(RunEnd::Exited(ExitCause::Failed)),
        "terminated" => Ok(RunEnd::Exited(ExitCause::Terminated)),
        "exited-cause-unknown" => Ok(RunEnd::Exited(ExitCause::Unknown)),
        "unverifiable" => Ok(RunEnd::Unverifiable),
        other => Err(unreadable("a run end", other)),
    }
}

pub(crate) fn integration_intent_token(intent: IntegrationIntent) -> &'static str {
    match intent {
        IntegrationIntent::Enabled => "enabled",
        IntegrationIntent::Disabled => "disabled",
    }
}

pub(crate) fn integration_intent_from_token(token: &str) -> Result<IntegrationIntent, FatalState> {
    match token {
        "enabled" => Ok(IntegrationIntent::Enabled),
        "disabled" => Ok(IntegrationIntent::Disabled),
        other => Err(unreadable("an integration intent", other)),
    }
}

/// Which provider file a repair fingerprint names.
///
/// Spelled per file rather than per provider: a provider that grows a second
/// integration surface gets a new token, and the old rows keep meaning the
/// file they were written about.
///
/// Written and matched, never decoded: a fingerprint's rows are found by
/// equality on the tokens the caller's own fingerprint renders, so nothing
/// turns a stored token back into a domain value. A reader belongs with the
/// first caller that needs to enumerate fingerprints rather than ask about
/// one.
pub(crate) fn config_target_token(target: ConfigTarget) -> &'static str {
    match target {
        ConfigTarget::ClaudeUserSettings => "claude-user-settings",
        ConfigTarget::CodexUserConfig => "codex-user-config",
    }
}

pub(crate) fn repairable_drift_token(drift: RepairableDrift) -> &'static str {
    match drift {
        RepairableDrift::Missing => "missing",
        RepairableDrift::OldRepresentation => "old-representation",
    }
}

/// What a command produced.
///
/// Deliberately not spelled the same as the `SessionCreated` *event* kind: one
/// names a fact in the log, the other names a receipt's outcome, and a shared
/// spelling would make a change to either look safe for both.
pub(crate) fn command_outcome_token(outcome: CommandOutcome) -> &'static str {
    match outcome {
        CommandOutcome::SessionCreated(_) => "created-session",
        CommandOutcome::RunStarted { .. } => "started-run",
    }
}

/// Which kind of thing a stored receipt names.
///
/// The two carry different targets, so reading one back is two questions and
/// not one: an unreadable token stops the read rather than defaulting to the
/// older kind, which would silently answer a continuation's retry with a
/// Session's first Run.
pub(crate) enum StoredOutcome {
    SessionCreated,
    RunStarted,
}

pub(crate) fn command_outcome_from_token(token: &str) -> Result<StoredOutcome, FatalState> {
    match token {
        "created-session" => Ok(StoredOutcome::SessionCreated),
        "started-run" => Ok(StoredOutcome::RunStarted),
        other => Err(unreadable("a command outcome", other)),
    }
}

/// Milliseconds since the Unix epoch, negative before it.
///
/// A clock far enough out to overflow this makes every recorded occurrence
/// time meaningless, so it fails rather than saturating into a plausible-
/// looking instant.
pub(crate) fn millis(at: SystemTime) -> Result<i64, FatalState> {
    let millis = match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()),
        Err(before) => i64::try_from(before.duration().as_millis()).map(|millis| -millis),
    };
    millis.map_err(|_| FatalState::UnrepresentableTime)
}

/// The same instant as the store would read it back.
///
/// The store keeps instants to the millisecond, so a value it hands a caller
/// has to be rounded to that resolution first — otherwise a receipt returned
/// by the write that created it never equals the receipt a retry reads back,
/// and "the same command returns the same receipt" quietly stops being true.
pub(crate) fn as_stored(at: SystemTime) -> Result<SystemTime, FatalState> {
    Ok(from_millis(millis(at)?))
}

pub(crate) fn from_millis(millis: i64) -> SystemTime {
    let magnitude = Duration::from_millis(millis.unsigned_abs());
    if millis < 0 {
        SystemTime::UNIX_EPOCH - magnitude
    } else {
        SystemTime::UNIX_EPOCH + magnitude
    }
}

pub(crate) fn unreadable(expected: &str, found: &str) -> FatalState {
    FatalState::Unreadable {
        detail: format!("{found:?} is not {expected}"),
    }
}

impl From<MalformedId> for FatalState {
    fn from(error: MalformedId) -> Self {
        Self::Unreadable {
            detail: error.to_string(),
        }
    }
}

#[cfg(test)]
#[path = "encoding_tests.rs"]
mod tests;
