use std::time::{Duration, SystemTime};

use super::*;

fn entry(id: &str, seconds_ago: u64) -> HistoryEntry {
    HistoryEntry {
        provider: KnownProvider::Claude,
        external_id: ExternalId::new(id).expect("usable"),
        last_active: SystemTime::UNIX_EPOCH + Duration::from_secs(10_000 - seconds_ago),
        store_label: "-w".to_owned(),
        path: std::path::PathBuf::from(format!("/store/{id}.jsonl")),
    }
}

/// A row keeps its id across passes: a list a person is looking at must not
/// renumber under them because the store was read again.
#[test]
fn a_row_keeps_its_id_across_passes() {
    let mut rows = HistoryRows::default();
    rows.replace(KnownProvider::Claude, vec![entry("a", 10)], Vec::new());
    let first = rows.rows()[0].session;
    rows.replace(
        KnownProvider::Claude,
        vec![entry("a", 5), entry("b", 1)],
        Vec::new(),
    );
    let listed = rows.rows();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].session, first, "the same identity, the same row");
    assert_eq!(listed[0].entry.external_id.as_str(), "b", "newest first");
}

/// A pass replaces its own provider's rows and leaves the other's alone.
#[test]
fn a_pass_replaces_only_its_provider() {
    let mut rows = HistoryRows::default();
    rows.replace(KnownProvider::Claude, vec![entry("a", 10)], Vec::new());
    let mut codex = entry("t", 1);
    codex.provider = KnownProvider::Codex;
    rows.replace(KnownProvider::Codex, vec![codex], Vec::new());
    rows.replace(KnownProvider::Claude, Vec::new(), Vec::new());
    let listed = rows.rows();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].entry.provider, KnownProvider::Codex);
}

/// A Session Corral already holds is decorated, never listed twice.
#[test]
fn a_known_session_is_decorated_rather_than_listed() {
    let mut rows = HistoryRows::default();
    let known = CorralSessionId::mint();
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    rows.replace(KnownProvider::Claude, Vec::new(), vec![(known, at)]);
    assert!(rows.rows().is_empty());
    assert_eq!(rows.last_active(known), Some(at));
    assert_eq!(rows.row(known), None);
}
