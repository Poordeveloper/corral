use super::*;

fn sqlite(result_code: i32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(result_code),
        Some("from the storage engine".to_owned()),
    )
}

/// Contention is the canonical transient condition. A store that concluded it
/// could no longer vouch because another writer held the file for a moment
/// would let one backup tool end the daemon permanently.
#[test]
fn contention_is_a_refusal_and_never_fatal() {
    for result_code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
        let error = StateError::from(sqlite(result_code));

        assert!(!error.is_fatal(), "{result_code} was treated as fatal");
        assert!(matches!(error, StateError::Refused(Refusal::Busy { .. })));
    }
}

/// A constraint violation is a write the engine rolled back whole, so the
/// store is exactly as it was.
#[test]
fn a_constraint_violation_is_a_refusal() {
    let error = StateError::from(sqlite(rusqlite::ffi::SQLITE_CONSTRAINT));

    assert!(!error.is_fatal());
    assert!(matches!(
        error,
        StateError::Refused(Refusal::Constraint { .. })
    ));
}

/// Everything the store cannot explain is fatal: once it stops being able to
/// say what happened, it stops vouching rather than retrying.
#[test]
fn an_unexplained_storage_failure_is_fatal() {
    for result_code in [
        rusqlite::ffi::SQLITE_CORRUPT,
        rusqlite::ffi::SQLITE_IOERR,
        rusqlite::ffi::SQLITE_NOTADB,
        rusqlite::ffi::SQLITE_FULL,
    ] {
        let error = StateError::from(sqlite(result_code));

        assert!(error.is_fatal(), "{result_code} was not treated as fatal");
        assert!(matches!(
            error,
            StateError::Fatal(FatalState::Storage { .. })
        ));
    }
}
