//! The projections: a summary of the log, and never more than that.
//!
//! Every mutation in this module is applied from an accepted durable event, so
//! clearing the projections and replaying the log reproduces them exactly. A
//! projection that could acquire a fact the log does not hold would destroy
//! that property, which is why nothing else writes to these tables (ADR 0002
//! D6, projection law).
//!
//! `RunAttached` and `RunDetached` deliberately change nothing here: whether a
//! runtime is attached right now is live state, and persisting it would record
//! runtime truth as a durable fact (AGENTS.md §Durable state).

use corral_core::{
    Binding, BindingId, BindingKey, BindingKind, CommandFingerprint, CommandId, CommandOutcome,
    CommandReceipt, CorralSessionId, Evidence, ExternalId, IdentityStatus, NodeId, Provenance,
    ProviderId, Run, RunId, RunOrdinal, Session, SessionLineage,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::encoding::StoredOutcome;
use crate::encoding::{
    assurance_from_token, assurance_token, binding_kind_from_token, binding_kind_token,
    command_outcome_from_token, command_outcome_token, evidence_source_from_token,
    evidence_source_token, from_millis, identity_status_from_token, identity_status_token, millis,
    provenance_from_token, provenance_token, run_end_from_token, run_end_token,
};
use crate::error::{FatalState, StateError};
use crate::event::SessionEvent;
use crate::schema;

/// Apply one fact, at the log position that holds it.
///
/// `accepted_seq` is the log's own `global_seq` for this fact — a durable
/// number, and the only ordering a projection may lean on: an implicit rowid
/// is renumbered by a VACUUM and reused after a delete, so a position derived
/// from one would stop matching what a replay produces.
pub(crate) fn apply(
    tx: &Transaction<'_>,
    event: &SessionEvent,
    accepted_seq: i64,
) -> Result<(), StateError> {
    match event {
        SessionEvent::SessionCreated {
            session,
            created_at,
        } => {
            tx.execute(
                "INSERT INTO sessions (id, created_at_ms) VALUES (?1, ?2)",
                params![session.to_string(), millis(*created_at)?],
            )?;
        }
        SessionEvent::BindingAdded(binding) => {
            tx.execute(
                "INSERT INTO bindings (
                     id, session_id, node_id, kind, provider, external_id, provenance,
                     assurance, identity_status, evidence_source, observed_at_ms, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    binding.id().to_string(),
                    binding.session().to_string(),
                    binding.key().node().to_string(),
                    binding_kind_token(binding.kind()),
                    binding.key().provider().as_str(),
                    binding.key().external_id().as_str(),
                    provenance_token(binding.provenance()),
                    assurance_token(binding.assurance()),
                    // Always `confirmed`: `BindingAdded` is the fact that
                    // Corral accepted this identity, and a contest is a
                    // separate later fact about it.
                    identity_status_token(binding.identity_status()),
                    evidence_source_token(binding.evidence().source()),
                    millis(binding.evidence().observed_at())?,
                    millis(binding.created_at())?,
                ],
            )?;
        }
        SessionEvent::BindingConfirmed {
            binding, evidence, ..
        } => {
            let changed = tx.execute(
                "UPDATE bindings
                    SET assurance = ?2, evidence_source = ?3, observed_at_ms = ?4
                  WHERE id = ?1",
                params![
                    binding.to_string(),
                    assurance_token(evidence.assurance()),
                    evidence_source_token(evidence.source()),
                    millis(evidence.observed_at())?,
                ],
            )?;
            expect_one(changed, "a binding to confirm")?;
        }
        // Only the identity status moves. The evidence that reached the
        // contest is recorded in the log; writing it over the binding's would
        // replace the evidence Corral still stands behind for the assurance it
        // keeps, and assurance stays orthogonal to a contest (ADR 0004 D8).
        SessionEvent::BindingContested { binding, .. } => {
            let changed = tx.execute(
                "UPDATE bindings SET identity_status = ?2 WHERE id = ?1",
                params![
                    binding.to_string(),
                    identity_status_token(IdentityStatus::Contested),
                ],
            )?;
            expect_one(changed, "a binding to contest")?;
        }
        SessionEvent::RunStarted {
            session,
            run,
            runtime_binding,
            started_at,
        } => {
            tx.execute(
                "INSERT INTO runs
                     (id, session_id, runtime_binding_id, accepted_seq, started_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    run.to_string(),
                    session.to_string(),
                    runtime_binding.to_string(),
                    accepted_seq,
                    started_at.map(millis).transpose()?,
                ],
            )?;
        }
        SessionEvent::RunEnded {
            run, end, ended_at, ..
        } => {
            let changed = tx.execute(
                "UPDATE runs SET end_state = ?2, ended_at_ms = ?3 WHERE id = ?1",
                params![
                    run.to_string(),
                    run_end_token(*end),
                    ended_at.map(millis).transpose()?,
                ],
            )?;
            expect_one(changed, "a run to end")?;
        }
        // Attachment is live state. The facts are kept in the log; the
        // projection stays silent about them on purpose.
        SessionEvent::RunAttached { .. } | SessionEvent::RunDetached { .. } => {}
        SessionEvent::SessionForkedFrom(lineage) => {
            tx.execute(
                "INSERT INTO session_lineage (child_session_id, parent_session_id, assurance)
                 VALUES (?1, ?2, ?3)",
                params![
                    lineage.child().to_string(),
                    lineage.parent().to_string(),
                    assurance_token(lineage.assurance()),
                ],
            )?;
        }
        SessionEvent::CommandAccepted {
            command,
            fingerprint,
            outcome,
            accepted_at,
        } => {
            let run = match outcome {
                CommandOutcome::SessionCreated(_) => None,
                CommandOutcome::RunStarted { run, .. } => Some(run.to_string()),
            };
            tx.execute(
                "INSERT INTO command_receipts (
                     command_id, fingerprint, outcome_kind, outcome_target, outcome_run,
                     accepted_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    command.as_str(),
                    fingerprint.as_str(),
                    command_outcome_token(*outcome),
                    outcome.session().to_string(),
                    run,
                    millis(*accepted_at)?,
                ],
            )?;
        }
    }
    Ok(())
}

/// Empty every projection, leaving the log and the store's own metadata alone.
pub(crate) fn clear(tx: &Transaction<'_>) -> Result<(), StateError> {
    for table in schema::PROJECTIONS {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
}

pub(crate) fn sessions(connection: &Connection) -> Result<Vec<Session>, StateError> {
    let mut statement =
        connection.prepare("SELECT id, created_at_ms FROM sessions ORDER BY created_at_ms, id")?;
    let rows = statement.query_map([], |row| Ok((text(row, 0)?, integer(row, 1)?)))?;
    let mut sessions = Vec::new();
    for row in rows {
        let (id, created_at_ms) = row?;
        sessions.push(Session::new(
            id.parse().map_err(FatalState::from)?,
            from_millis(created_at_ms),
        ));
    }
    Ok(sessions)
}

pub(crate) fn session(
    connection: &Connection,
    id: CorralSessionId,
) -> Result<Option<Session>, StateError> {
    let row: Option<i64> = connection
        .prepare_cached("SELECT created_at_ms FROM sessions WHERE id = ?1")?
        .query_row(params![id.to_string()], |row| row.get(0))
        .optional()?;
    Ok(row.map(|created_at_ms| Session::new(id, from_millis(created_at_ms))))
}

pub(crate) fn binding(
    connection: &Connection,
    id: BindingId,
) -> Result<Option<Binding>, StateError> {
    query_binding(connection, "WHERE id = ?1", params![id.to_string()])
}

pub(crate) fn binding_by_key(
    connection: &Connection,
    key: &BindingKey,
) -> Result<Option<Binding>, StateError> {
    query_binding(
        connection,
        "WHERE node_id = ?1 AND provider = ?2 AND external_id = ?3 AND kind = ?4",
        params![
            key.node().to_string(),
            key.provider().as_str(),
            key.external_id().as_str(),
            binding_kind_token(key.kind()),
        ],
    )
}

/// The binding this Session may currently drive control through, if any.
///
/// Answers the at-most-one rule without materializing every binding the
/// Session has: the check looks at kind and assurance, and never at the names
/// a full decode would re-validate.
pub(crate) fn control_capable_runtime_binding(
    connection: &Connection,
    session: CorralSessionId,
    excluding: Option<BindingId>,
) -> Result<Option<BindingId>, StateError> {
    // One query with a self-comparison standing in for "exclude nothing",
    // rather than a second copy of the loop below. The two callers ask the
    // same question — admitting a candidate excludes it, a continuation has
    // none to exclude — and a fix applied to one copy that silently missed the
    // other would be a fix applied to neither.
    let excluded = excluding.map_or_else(String::new, |binding| binding.to_string());
    let mut statement = connection.prepare_cached(
        "SELECT id, assurance FROM bindings
          WHERE session_id = ?1 AND kind = ?2 AND id != ?3",
    )?;
    let rows = statement.query_map(
        params![
            session.to_string(),
            binding_kind_token(BindingKind::Runtime),
            excluded,
        ],
        |row| Ok((text(row, 0)?, text(row, 1)?)),
    )?;
    for row in rows {
        let (id, assurance) = row?;
        if assurance_from_token(&assurance)?.permits_control() {
            return Ok(Some(id.parse().map_err(FatalState::from)?));
        }
    }
    Ok(None)
}

pub(crate) fn bindings_of(
    connection: &Connection,
    session: CorralSessionId,
) -> Result<Vec<Binding>, StateError> {
    let mut statement = connection.prepare(&format!(
        "{BINDING_COLUMNS} WHERE session_id = ?1 ORDER BY created_at_ms, id"
    ))?;
    let rows = statement.query_map(params![session.to_string()], binding_row)?;
    let mut bindings = Vec::new();
    for row in rows {
        bindings.push(binding_from(row?)?);
    }
    Ok(bindings)
}

/// A Session's Runs, oldest episode first, each numbered by where it sits.
///
/// The ordinal is read off this order rather than stored: it is a position
/// within the Session (ADR 0002 D1), so a Run whose start is learned late
/// takes the place its occurrence earns instead of the place its acceptance
/// happened to fall in. Runs whose start the runtime could not state come
/// last, ordered by when the log accepted them, because nothing better is
/// known about them.
pub(crate) fn runs_of(
    connection: &Connection,
    session: CorralSessionId,
) -> Result<Vec<Run>, StateError> {
    let mut statement = connection.prepare(&format!(
        "{RUN_COLUMNS} WHERE session_id = ?1
         ORDER BY started_at_ms IS NULL, started_at_ms, accepted_seq"
    ))?;
    let rows = statement.query_map(params![session.to_string()], run_row)?;
    let mut runs = Vec::new();
    for (position, row) in rows.enumerate() {
        let run = run_from(row?)?;
        // A number too large to hold leaves the Run unnumbered. The ordinal is
        // display data (ADR 0002 D1); a Session with four billion Runs is a
        // presentation problem, not a reason to stop vouching for the store.
        runs.push(match u32::try_from(position.saturating_add(1)) {
            Ok(position) => run.with_ordinal(RunOrdinal::from_position(position)),
            Err(_) => run,
        });
    }
    Ok(runs)
}

/// The Run as the log knows it, or `None` when the log does not know it at
/// all.
///
/// Callers hold Runs as live state, so this is what says whether a fact about
/// one may be recorded — and it is where the Run's Session comes from, rather
/// than from the caller's copy, so a fact can never be filed under a Session
/// the store does not agree with.
pub(crate) fn recorded_run(connection: &Connection, id: RunId) -> Result<Option<Run>, StateError> {
    let row = connection
        .query_row(
            &format!("{RUN_COLUMNS} WHERE id = ?1"),
            params![id.to_string()],
            run_row,
        )
        .optional()?;
    row.map(run_from).transpose()
}

const RUN_COLUMNS: &str = "SELECT id, session_id, runtime_binding_id, started_at_ms, \
                           ended_at_ms, end_state FROM runs";

type RunRow = (
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

fn run_row(row: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok((
        text(row, 0)?,
        text(row, 1)?,
        text(row, 2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn run_from(row: RunRow) -> Result<Run, StateError> {
    let (id, session, binding, started, ended, end_state) = row;
    let run = Run::started(
        id.parse().map_err(FatalState::from)?,
        session.parse().map_err(FatalState::from)?,
        binding.parse().map_err(FatalState::from)?,
        occurrence(started),
    );
    Ok(match end_state {
        None => run,
        Some(token) => run.ended(run_end_from_token(&token)?, occurrence(ended)),
    })
}

/// The Run a Session's creating command produced.
///
/// Ordered by acceptance, deliberately not by occurrence the way `runs_of`
/// presents Runs to a person. What this answers is "which Run did that command
/// write", and a Session's creation and its first `RunStarted` are one
/// transaction — so the earliest accepted Run is that one, unconditionally. A
/// later Run carrying an earlier occurrence time is an ordinary thing (a clock
/// that stepped back, a Run appended after its association was confirmed), and
/// ordering by occurrence would hand a retry the wrong episode.
pub(crate) fn first_run_of(
    connection: &Connection,
    session: CorralSessionId,
) -> Result<Option<RunId>, StateError> {
    let found: Option<String> = connection
        .prepare_cached("SELECT id FROM runs WHERE session_id = ?1 ORDER BY accepted_seq LIMIT 1")?
        .query_row(params![session.to_string()], |row| row.get(0))
        .optional()?;
    found
        .map(|id| id.parse().map_err(|error| FatalState::from(error).into()))
        .transpose()
}

/// Every unfinished Run that Corral owns as a managed-runtime episode on this
/// node.
///
/// The predicate is ownership, never `ended_at IS NULL` alone: a discovered or
/// provider-owned Run may legitimately outlive a `corrald` restart, and
/// closing one would be Corral asserting an end to something it never managed
/// (grill Q5). What identifies a managed episode is its binding — created by
/// Corral, in the reserved provider namespace, on this node (ADR 0008 D1).
pub(crate) fn open_managed_runs(
    connection: &Connection,
    node: NodeId,
) -> Result<Vec<(CorralSessionId, RunId)>, StateError> {
    let mut statement = connection.prepare(
        "SELECT runs.session_id, runs.id FROM runs
           JOIN bindings ON bindings.id = runs.runtime_binding_id
          WHERE runs.end_state IS NULL
            AND bindings.node_id = ?1
            AND bindings.kind = ?2
            AND bindings.provider = ?3
            AND bindings.provenance = ?4
          ORDER BY runs.accepted_seq",
    )?;
    let rows = statement.query_map(
        params![
            node.to_string(),
            binding_kind_token(BindingKind::Runtime),
            ProviderId::RESERVED_FOR_CORRAL,
            provenance_token(Provenance::CorralCreated),
        ],
        |row| Ok((text(row, 0)?, text(row, 1)?)),
    )?;
    let mut open = Vec::new();
    for row in rows {
        let (session, run) = row?;
        open.push((
            session.parse().map_err(FatalState::from)?,
            run.parse().map_err(FatalState::from)?,
        ));
    }
    Ok(open)
}

/// The episode this runtime binding is currently running, if any.
///
/// A runtime binding names one runtime, and one runtime runs one episode at a
/// time — so this is what a second `RunStarted` under the same binding has to
/// find empty.
pub(crate) fn live_run_of_binding(
    connection: &Connection,
    binding: BindingId,
) -> Result<Option<RunId>, StateError> {
    let found: Option<String> = connection
        .prepare_cached("SELECT id FROM runs WHERE runtime_binding_id = ?1 AND end_state IS NULL")?
        .query_row(params![binding.to_string()], |row| row.get(0))
        .optional()?;
    found
        .map(|id| id.parse().map_err(|error| FatalState::from(error).into()))
        .transpose()
}

pub(crate) fn lineage_of(
    connection: &Connection,
    child: CorralSessionId,
) -> Result<Option<SessionLineage>, StateError> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT parent_session_id, assurance FROM session_lineage WHERE child_session_id = ?1",
            params![child.to_string()],
            |row| Ok((text(row, 0)?, text(row, 1)?)),
        )
        .optional()?;
    let Some((parent, assurance)) = row else {
        return Ok(None);
    };
    SessionLineage::record(
        child,
        parent.parse().map_err(FatalState::from)?,
        assurance_from_token(&assurance)?,
    )
    .map(Some)
    .map_err(|refusal| {
        FatalState::Unreadable {
            detail: refusal.to_string(),
        }
        .into()
    })
}

pub(crate) fn receipt(
    connection: &Connection,
    command: &CommandId,
) -> Result<Option<CommandReceipt>, StateError> {
    let row: Option<(String, String, String, Option<String>, i64)> = connection
        .query_row(
            "SELECT fingerprint, outcome_kind, outcome_target, outcome_run, accepted_at_ms
               FROM command_receipts WHERE command_id = ?1",
            params![command.as_str()],
            |row| {
                Ok((
                    text(row, 0)?,
                    text(row, 1)?,
                    text(row, 2)?,
                    row.get(3)?,
                    integer(row, 4)?,
                ))
            },
        )
        .optional()?;
    let Some((fingerprint, outcome_kind, outcome_target, outcome_run, accepted_at_ms)) = row else {
        return Ok(None);
    };
    let session = outcome_target.parse().map_err(FatalState::from)?;
    let outcome = match command_outcome_from_token(&outcome_kind)? {
        StoredOutcome::SessionCreated => CommandOutcome::SessionCreated(session),
        // A receipt of this kind without the Run it produced is a projection
        // that cannot answer the retry it exists to answer. Minting one to
        // cover the hole would answer a question about a past that is not
        // there.
        StoredOutcome::RunStarted => CommandOutcome::RunStarted {
            session,
            run: outcome_run
                .ok_or_else(|| FatalState::Unreadable {
                    detail: format!(
                        "the receipt for command {} started a run and names none",
                        command.as_str()
                    ),
                })?
                .parse()
                .map_err(FatalState::from)?,
        },
    };
    Ok(Some(CommandReceipt::new(
        command.clone(),
        CommandFingerprint::from_canonical(fingerprint),
        outcome,
        from_millis(accepted_at_ms),
    )))
}

const BINDING_COLUMNS: &str = "SELECT id, session_id, node_id, kind, provider, external_id, \
                               provenance, assurance, identity_status, evidence_source, \
                               observed_at_ms, created_at_ms FROM bindings";

type BindingRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
);

fn query_binding(
    connection: &Connection,
    filter: &str,
    parameters: impl rusqlite::Params,
) -> Result<Option<Binding>, StateError> {
    let row: Option<BindingRow> = connection
        .query_row(
            &format!("{BINDING_COLUMNS} {filter}"),
            parameters,
            binding_row,
        )
        .optional()?;
    row.map(binding_from).transpose()
}

fn binding_row(row: &Row<'_>) -> rusqlite::Result<BindingRow> {
    Ok((
        text(row, 0)?,
        text(row, 1)?,
        text(row, 2)?,
        text(row, 3)?,
        text(row, 4)?,
        text(row, 5)?,
        text(row, 6)?,
        text(row, 7)?,
        text(row, 8)?,
        text(row, 9)?,
        integer(row, 10)?,
        integer(row, 11)?,
    ))
}

fn binding_from(row: BindingRow) -> Result<Binding, StateError> {
    let (
        id,
        session,
        node,
        kind,
        provider,
        external_id,
        provenance,
        assurance,
        identity_status,
        evidence_source,
        observed_at_ms,
        created_at_ms,
    ) = row;
    let binding = Binding::new(
        id.parse().map_err(FatalState::from)?,
        session.parse().map_err(FatalState::from)?,
        BindingKey::new(
            node.parse().map_err(FatalState::from)?,
            binding_kind_from_token(&kind)?,
            ProviderId::new(provider).map_err(malformed)?,
            ExternalId::new(external_id).map_err(malformed)?,
        ),
        provenance_from_token(&provenance)?,
        Evidence::new(
            evidence_source_from_token(&evidence_source)?,
            assurance_from_token(&assurance)?,
            from_millis(observed_at_ms),
        ),
        from_millis(created_at_ms),
    );
    Ok(match identity_status_from_token(&identity_status)? {
        IdentityStatus::Confirmed => binding,
        IdentityStatus::Contested => binding.contested(),
    })
}

/// A stored instant is authoritative by construction: a first-observed time is
/// never written as an occurrence time, so nothing read back can be one
/// (ADR 0002 D6).
fn occurrence(stored: Option<i64>) -> corral_core::OccurrenceTime {
    match stored {
        Some(at) => corral_core::OccurrenceTime::Authoritative(from_millis(at)),
        // The fact happened; the runtime could not say when.
        None => corral_core::OccurrenceTime::Unknown,
    }
}

/// A projection update that matches nothing means the log implies a row the
/// projections do not hold. That is not a write to retry; it is a store whose
/// projections can no longer be trusted.
fn expect_one(changed: usize, expected: &str) -> Result<(), StateError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(FatalState::Unreadable {
            detail: format!("the log names {expected} that the projections do not hold"),
        }
        .into())
    }
}

fn malformed(error: corral_core::MalformedExternalName) -> StateError {
    FatalState::Unreadable {
        detail: error.to_string(),
    }
    .into()
}

fn text(row: &Row<'_>, index: usize) -> rusqlite::Result<String> {
    row.get(index)
}

fn integer(row: &Row<'_>, index: usize) -> rusqlite::Result<i64> {
    row.get(index)
}
