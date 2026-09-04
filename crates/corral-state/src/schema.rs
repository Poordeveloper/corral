//! The registry store's durable shape, and the checks that decide whether it
//! may be trusted.
//!
//! `schema_version` and `node_identity` are the store's own metadata, not
//! projections: they are not derived from any event and a projection rebuild
//! never touches them. Everything else in here is either the durable semantic
//! event log or a projection of it.

use std::path::Path;
use std::time::Duration;

use corral_core::NodeId;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::encoding;
use crate::error::{FatalState, StateError};

/// The only schema this build knows.
///
/// No migration exists yet: `STORAGE_EPOCH` is `dev`, development databases
/// are disposable, and the first migration is written by the change that
/// needs one. A store at any other version is refused rather than guessed at.
pub(crate) const SCHEMA_VERSION: u32 = 5;

/// How long an operation waits for another writer before giving up.
///
/// Corral runs one primary daemon per account, so contention means something
/// unusual — a backup tool, a tool inspecting the store, a departing daemon.
/// Waiting is the right answer to all three; concluding the store is broken is
/// not.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The durable shape. Written once, on a store that has none.
///
/// `session_events` is the log: `seq` is per-Session and monotonic, and
/// `global_seq` is the order Corral accepted facts across every Session, which
/// is what a replay must follow so a lineage edge is never applied before the
/// Session it names.
///
/// `session_lineage.parent_session_id` deliberately carries no foreign key: an
/// edge naming a deleted Session is kept as a recorded fact with an
/// unresolvable target rather than silently erased (ADR 0002 D8).
const DEFINITION: &str = "\
CREATE TABLE schema_version (
    only_row INTEGER PRIMARY KEY CHECK (only_row = 0),
    version  INTEGER NOT NULL
) STRICT;

CREATE TABLE node_identity (
    only_row INTEGER PRIMARY KEY CHECK (only_row = 0),
    node_id  TEXT NOT NULL
) STRICT;

CREATE TABLE session_events (
    global_seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id     TEXT NOT NULL,
    seq            INTEGER NOT NULL,
    kind           TEXT NOT NULL,
    payload        TEXT NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    UNIQUE (session_id, seq)
) STRICT;

CREATE TABLE sessions (
    id            TEXT PRIMARY KEY,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE bindings (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES sessions(id),
    node_id         TEXT NOT NULL,
    kind            TEXT NOT NULL,
    provider        TEXT NOT NULL,
    external_id     TEXT NOT NULL,
    provenance      TEXT NOT NULL,
    assurance       TEXT NOT NULL,
    -- Whether Corral still stands behind the identity this edge names. A
    -- column rather than a payload field: it is derived from the log —
    -- `binding-added` writes `confirmed`, `binding-contested` writes
    -- `contested` — so a replay reproduces it exactly (ADR 0004 D8).
    identity_status TEXT NOT NULL,
    evidence_source TEXT NOT NULL,
    observed_at_ms  INTEGER NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    UNIQUE (node_id, provider, external_id, kind)
) STRICT;

CREATE TABLE runs (
    id                 TEXT PRIMARY KEY,
    session_id         TEXT NOT NULL REFERENCES sessions(id),
    runtime_binding_id TEXT NOT NULL REFERENCES bindings(id),
    -- The log position that put this row here. A Run's place in its Session is
    -- read off occurrence time, and this settles ties between Runs whose start
    -- the runtime could not state — from a fact the log holds, not from an
    -- implicit rowid a VACUUM may renumber.
    accepted_seq       INTEGER NOT NULL,
    started_at_ms      INTEGER,
    -- Where Corral started this episode. NULL for a Run Corral found rather
    -- than launched, and for one whose path this store cannot hold: the
    -- directory is unknown, which refuses a continuation rather than choosing
    -- one (Q35).
    working_directory  TEXT,
    ended_at_ms        INTEGER,
    end_state          TEXT
) STRICT;

CREATE INDEX bindings_by_session ON bindings (session_id);
CREATE INDEX runs_by_session ON runs (session_id);
-- Every `RunStarted` asks whether this binding is already running an episode,
-- under the writer lock, and the runs projection is never compacted.
CREATE INDEX runs_by_binding ON runs (runtime_binding_id);

CREATE TABLE session_lineage (
    child_session_id  TEXT PRIMARY KEY REFERENCES sessions(id),
    parent_session_id TEXT NOT NULL,
    assurance         TEXT NOT NULL
) STRICT;

-- What the user chose about provider integration, and the bounded self-repair
-- authority that choice grants. Corral-owned facts with no external source of
-- truth: the provider's own file cannot carry either, because an entry missing
-- from it is indistinguishable from a provider rewrite (ADR 0013 D5/D6).
--
-- Node-scoped without a node column: one store belongs to one node, the way
-- `node_identity` is one row. A provider's integration is a fact about this
-- machine's configuration, so a second node's answer could never belong here.
--
-- Not projections. Nothing derives them from the event log, and a rebuild must
-- leave them untouched — replaying the log would otherwise forget that the
-- user turned integration off.
CREATE TABLE integration_intent (
    provider      TEXT PRIMARY KEY,
    intent        TEXT NOT NULL,
    changed_at_ms INTEGER NOT NULL
) STRICT;

-- One row per automatic repair Corral performed, so the rolling window can be
-- counted after a restart. Rows outside every window are pruned when their
-- fingerprint is next examined, never on a timer.
CREATE TABLE integration_repairs (
    provider       TEXT NOT NULL,
    config_target  TEXT NOT NULL,
    drift_class    TEXT NOT NULL,
    repaired_at_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX integration_repairs_by_fingerprint
    ON integration_repairs (provider, config_target, drift_class);

-- An open circuit breaker: repeated evidence that another authority keeps
-- undoing Corral's integration. Sticky on purpose — the row carries no expiry,
-- because the rolling window decides only when authority is withdrawn, never
-- when it returns. Only an explicit user reconciliation deletes it, and a
-- daemon restart must not (grill Q4′).
CREATE TABLE integration_breakers (
    provider      TEXT NOT NULL,
    config_target TEXT NOT NULL,
    drift_class   TEXT NOT NULL,
    opened_at_ms  INTEGER NOT NULL,
    PRIMARY KEY (provider, config_target, drift_class)
) STRICT;

CREATE TABLE command_receipts (
    command_id     TEXT PRIMARY KEY,
    fingerprint    TEXT NOT NULL,
    outcome_kind   TEXT NOT NULL,
    -- The Session every outcome names. Which Session a command acted on is the
    -- one thing both kinds of receipt answer.
    outcome_target TEXT NOT NULL,
    -- The Run a continuation produced, and NULL for an outcome that names no
    -- Run. Absence here is the kind saying so, never a Run that was lost.
    outcome_run    TEXT,
    accepted_at_ms INTEGER NOT NULL
) STRICT;
";

/// The projections, in an order that lets them be emptied without tripping a
/// foreign key. Replay does not use this order — it follows `global_seq`,
/// which already places a row after whatever it references.
///
/// The `integration_*` tables are absent deliberately: they hold Corral-owned
/// facts no event derives, so a rebuild must leave them alone.
pub(crate) const PROJECTIONS: [&str; 5] = [
    "session_lineage",
    "runs",
    "bindings",
    "sessions",
    "command_receipts",
];

/// Open the store and decide whether it may be used at all.
///
/// Nothing here is best-effort: a store that cannot be opened, cannot be
/// initialized, or fails its integrity check is a startup failure, because a
/// daemon that serves state-derived claims from an unusable store is worse
/// than one that does not start (ADR 0002, Q14).
pub(crate) fn open(path: &Path) -> Result<(Connection, NodeId), StateError> {
    let unopenable = |detail: String| FatalState::Unopenable {
        path: path.to_path_buf(),
        detail,
    };

    // This is the owner the lint names: the registry store is opened here and
    // nowhere else, so one log has one writer.
    #[allow(clippy::disallowed_methods)]
    let mut connection = Connection::open(path).map_err(|source| unopenable(source.to_string()))?;
    // Referential integrity is the store's own rule, enforced by the store:
    // duplicating it as Rust pre-checks would leave two owners of one
    // invariant, and the transaction rollback it triggers is what keeps an
    // event and its projection from ever landing apart.
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|source| unopenable(source.to_string()))?;

    // A momentary lock is not this daemon's answer to give up on: without a
    // wait, one concurrent writer turns every operation into a hard failure.
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|source| unopenable(source.to_string()))?;

    // `quick_check` rather than `integrity_check`: it catches the structural
    // damage and the "this is not a database" case that make a store unusable,
    // without reading every page. The log is append-only, so a full check would
    // make every cold start slower than the last one forever.
    let integrity: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|source| unopenable(source.to_string()))?;
    if integrity != "ok" {
        return Err(unopenable(integrity).into());
    }

    let node = initialize(&mut connection).map_err(|error| match error {
        StateError::Fatal(FatalState::Storage { detail }) => unopenable(detail).into(),
        other => other,
    })?;
    Ok((connection, node))
}

/// Create the schema on an empty store, and read back the identity of one that
/// already exists.
///
/// Creation is one transaction, tables and identity together. Split across
/// two, a crash in between would leave a store with tables and no identity —
/// a shape neither branch below can interpret, and one no later start could
/// repair.
fn initialize(connection: &mut Connection) -> Result<NodeId, StateError> {
    let mismatch = |found| FatalState::SchemaVersionMismatch {
        expected: SCHEMA_VERSION,
        found,
    };
    match stored_schema(connection)? {
        StoredSchema::Version(SCHEMA_VERSION) => return stored_node(connection),
        StoredSchema::Version(found) => return Err(mismatch(Some(found)).into()),
        StoredSchema::VersionLost => return Err(mismatch(None).into()),
        StoredSchema::NotARegistry => {
            return Err(FatalState::Storage {
                detail: "the file is a database, but not Corral's registry".to_owned(),
            }
            .into());
        }
        StoredSchema::NeverWritten => {}
    }

    let node = NodeId::mint();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(DEFINITION)?;
    transaction.execute(
        "INSERT INTO schema_version (only_row, version) VALUES (0, ?1)",
        [SCHEMA_VERSION],
    )?;
    transaction.execute(
        "INSERT INTO node_identity (only_row, node_id) VALUES (0, ?1)",
        [node.to_string()],
    )?;
    transaction.commit()?;
    Ok(node)
}

/// Confirm the store is still the schema and the store this process validated.
///
/// Run before every read and every write, because a store that was replaced or
/// rewritten underneath the daemon invalidates every fact read since — and a
/// daemon holding durable state has no other moment to notice.
///
/// Two cached single-row lookups. Whether the store *could* be initialized is a
/// question only `open` asks: by the time anything vouches the answer is
/// settled, and re-asking it here would recompile a `sqlite_schema` scan on
/// every operation.
pub(crate) fn vouch(connection: &Connection, node: NodeId) -> Result<(), StateError> {
    let version: Option<u32> = connection
        .prepare_cached("SELECT version FROM schema_version WHERE only_row = 0")?
        .query_row([], |row| row.get(0))
        .optional()?;
    if version != Some(SCHEMA_VERSION) {
        return Err(FatalState::SchemaVersionMismatch {
            expected: SCHEMA_VERSION,
            found: version,
        }
        .into());
    }

    let found: Option<String> = connection
        .prepare_cached("SELECT node_id FROM node_identity WHERE only_row = 0")?
        .query_row([], |row| row.get(0))
        .optional()?;
    if found.as_deref() != Some(node.to_string().as_str()) {
        return Err(FatalState::StoreIdentityChanged {
            expected: node,
            found,
        }
        .into());
    }
    Ok(())
}

/// What a store on disk says about its own schema.
enum StoredSchema {
    /// No `schema_version` table and nothing else either. Nothing has ever
    /// been written here, and this is the only state that may be initialized.
    NeverWritten,
    /// A perfectly good database that is not Corral's registry.
    NotARegistry,
    Version(u32),
    /// Corral's tables are there and the version is not — a store whose own
    /// version was lost. Never an empty store: creating the schema over it
    /// would fail on tables that are already there, and reinterpreting it would
    /// read facts written under an unknown version as if they were this one.
    VersionLost,
}

fn stored_schema(connection: &Connection) -> Result<StoredSchema, StateError> {
    let table: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if table.is_none() {
        // A file with tables but no `schema_version` was written by
        // something. Creating the registry inside it would either fail on a
        // name collision or leave the authoritative store sharing a file with
        // unrelated data — the same hazard §5 names when it keeps the registry
        // and the history index apart, applied here by analogy rather than by
        // quotation.
        let tables: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if tables == 0 {
            return Ok(StoredSchema::NeverWritten);
        }
        // Corral's own tables without its version row is a damaged registry,
        // not a stranger's file. Telling an operator to look at the wrong file
        // is the worst answer available on a fail-closed startup path.
        // `session_events` and nothing else: `sessions` is one of the most
        // common table names there is, and mistaking a web framework's session
        // store for a damaged registry points an operator at the wrong file
        // just as surely as the reverse.
        let ours: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema
              WHERE type = 'table' AND name = 'session_events'",
            [],
            |row| row.get(0),
        )?;
        return Ok(if ours > 0 {
            StoredSchema::VersionLost
        } else {
            StoredSchema::NotARegistry
        });
    }
    let version: Option<u32> = connection
        .query_row(
            "SELECT version FROM schema_version WHERE only_row = 0",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match version {
        Some(version) => StoredSchema::Version(version),
        None => StoredSchema::VersionLost,
    })
}

fn stored_node(connection: &Connection) -> Result<NodeId, StateError> {
    let stored: String = connection.query_row(
        "SELECT node_id FROM node_identity WHERE only_row = 0",
        [],
        |row| row.get(0),
    )?;
    stored
        .parse()
        .map_err(|_| encoding::unreadable("a node id", &stored).into())
}
