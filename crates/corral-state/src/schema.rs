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
pub(crate) const SCHEMA_VERSION: u32 = 1;

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
    evidence_source TEXT NOT NULL,
    observed_at_ms  INTEGER NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    UNIQUE (node_id, provider, external_id, kind)
) STRICT;

CREATE TABLE runs (
    id                 TEXT PRIMARY KEY,
    session_id         TEXT NOT NULL REFERENCES sessions(id),
    runtime_binding_id TEXT NOT NULL REFERENCES bindings(id),
    ordinal            INTEGER NOT NULL,
    started_at_ms      INTEGER,
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

CREATE TABLE command_receipts (
    command_id     TEXT PRIMARY KEY,
    fingerprint    TEXT NOT NULL,
    outcome_kind   TEXT NOT NULL,
    outcome_target TEXT NOT NULL,
    accepted_at_ms INTEGER NOT NULL
) STRICT;
";

/// The projections, in an order that lets them be emptied without tripping a
/// foreign key. Replay does not use this order — it follows `global_seq`,
/// which already places a row after whatever it references.
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
    /// The table exists and states nothing — a store whose own version was
    /// lost. Never an empty store: creating the schema over it would fail on a
    /// table that is already there, and reinterpreting it would read facts
    /// written under an unknown version as if they were this one.
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
        // A file with tables but no `schema_version` is not an unwritten
        // registry — it is somebody else's database. Creating the registry
        // inside it would put the authoritative store and something unrelated
        // in one file, which `ARCHITECTURE.md` §5 forbids outright.
        let foreign: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if foreign > 0 {
            return Ok(StoredSchema::NotARegistry);
        }
        return Ok(StoredSchema::NeverWritten);
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
