//! How a domain value is written to disk.
//!
//! Every token here is durable vocabulary: once a store has been written with
//! one, changing it rewrites the meaning of recorded facts. They are spelled
//! out rather than derived from Rust names so that renaming a variant is a
//! compile error here instead of a silent reinterpretation of the log.

use std::time::{Duration, SystemTime};

use corral_core::{
    Assurance, BindingKind, EvidenceSource, ExitCause, MalformedId, Provenance, RunEnd,
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
        "screen-detection" => Ok(EvidenceSource::ScreenDetection),
        "history-record" => Ok(EvidenceSource::HistoryRecord),
        "correlation" => Ok(EvidenceSource::Correlation),
        "user-assertion" => Ok(EvidenceSource::UserAssertion),
        other => Err(unreadable("an evidence source", other)),
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
