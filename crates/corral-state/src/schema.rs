//! The registry store's durable shape, and the checks that decide whether it
//! may be trusted.
//!
//! `schema_version` and `node_identity` are the store's own metadata, not
//! projections: they are not derived from any event and a projection rebuild
//! never touches them. Everything else in here is either the durable semantic
//! event log or a projection of it.

use std::path::Path;

use corral_core::NodeId;
use rusqlite::{Connection, OptionalExtension};

use crate::encoding;
use crate::error::{FatalState, StateError};

/// The only schema this build knows.
///
/// No migration exists yet: `STORAGE_EPOCH` is `dev`, development databases
/// are disposable, and the first migration is written by the change that
/// needs one. A store at any other version is refused rather than guessed at.
pub(crate) const SCHEMA_VERSION: u32 = 1;

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

/// The projections, in an order that satisfies their references both when
/// clearing and, reversed, when replaying.
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

    let connection = Connection::open(path).map_err(|source| unopenable(source.to_string()))?;
    // Referential integrity is the store's own rule, enforced by the store:
    // duplicating it as Rust pre-checks would leave two owners of one
    // invariant, and the transaction rollback it triggers is what keeps an
    // event and its projection from ever landing apart.
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|source| unopenable(source.to_string()))?;

    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|source| unopenable(source.to_string()))?;
    if integrity != "ok" {
        return Err(unopenable(integrity).into());
    }

    let node = initialize(&connection).map_err(|error| match error {
        StateError::Fatal(FatalState::Storage { detail }) => unopenable(detail).into(),
        other => other,
    })?;
    Ok((connection, node))
}

/// Create the schema on an empty store, and read back the identity of one that
/// already exists.
fn initialize(connection: &Connection) -> Result<NodeId, StateError> {
    let existing: Option<u32> = stored_version(connection)?;
    match existing {
        None => {
            let node = NodeId::mint();
            connection.execute_batch(&format!("BEGIN;\n{DEFINITION}\nCOMMIT;"))?;
            connection.execute(
                "INSERT INTO schema_version (only_row, version) VALUES (0, ?1)",
                [SCHEMA_VERSION],
            )?;
            connection.execute(
                "INSERT INTO node_identity (only_row, node_id) VALUES (0, ?1)",
                [node.to_string()],
            )?;
            Ok(node)
        }
        Some(SCHEMA_VERSION) => stored_node(connection),
        found => Err(FatalState::SchemaVersionMismatch {
            expected: SCHEMA_VERSION,
            found,
        }
        .into()),
    }
}

/// Confirm the store is still the schema and the store this process validated.
///
/// Run before every read and every write, because a store that was replaced or
/// rewritten underneath the daemon invalidates every fact read since — and a
/// daemon holding durable state has no other moment to notice. Two indexed
/// single-row lookups.
pub(crate) fn vouch(connection: &Connection, node: NodeId) -> Result<(), StateError> {
    match stored_version(connection)? {
        Some(SCHEMA_VERSION) => {}
        found => {
            return Err(FatalState::SchemaVersionMismatch {
                expected: SCHEMA_VERSION,
                found,
            }
            .into());
        }
    }

    let found: Option<String> = connection
        .query_row(
            "SELECT node_id FROM node_identity WHERE only_row = 0",
            [],
            |row| row.get(0),
        )
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

/// `None` on a store that has never been written, which is the only case that
/// may be initialized. A `schema_version` table that exists but cannot be read
/// is a failure, never an empty store.
fn stored_version(connection: &Connection) -> Result<Option<u32>, StateError> {
    let table: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if table.is_none() {
        return Ok(None);
    }
    Ok(connection
        .query_row(
            "SELECT version FROM schema_version WHERE only_row = 0",
            [],
            |row| row.get(0),
        )
        .optional()?)
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
