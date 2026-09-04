use std::time::{Duration, SystemTime};

use super::*;

/// An entry at an exact time, for the resolved half of a pass.
fn at_entry(id: &str, at: SystemTime) -> HistoryEntry {
    let mut entry = entry(id, 0);
    entry.last_active = at;
    entry
}

fn entry(id: &str, seconds_ago: u64) -> HistoryEntry {
    HistoryEntry {
        provider: KnownProvider::Claude,
        external_id: ExternalId::new(id).expect("usable"),
        last_active: SystemTime::UNIX_EPOCH + Duration::from_secs(10_000 - seconds_ago),
        observed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(10_000),
        store_label: "-w".to_owned(),
        path: std::path::PathBuf::from(format!("/store/{id}.jsonl")),
    }
}

/// A row keeps its id across passes: a list a person is looking at must not
/// renumber under them because the store was read again.
#[test]
fn a_row_keeps_its_id_across_passes() {
    let mut rows = HistoryRows::default();
    rows.replace(KnownProvider::Claude, vec![entry("a", 10)], Vec::new(), 0);
    let first = rows.rows()[0].session;
    rows.replace(
        KnownProvider::Claude,
        vec![entry("a", 5), entry("b", 1)],
        Vec::new(),
        0,
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
    rows.replace(KnownProvider::Claude, vec![entry("a", 10)], Vec::new(), 0);
    let mut codex = entry("t", 1);
    codex.provider = KnownProvider::Codex;
    rows.replace(KnownProvider::Codex, vec![codex], Vec::new(), 0);
    rows.replace(KnownProvider::Claude, Vec::new(), Vec::new(), 0);
    let listed = rows.rows();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].entry.provider, KnownProvider::Codex);
}

/// A Session Corral already holds keeps its own id and mints nothing, and it
/// is a row here under that id — the caller drops it when a live tier is
/// already showing it, and shows it when none is. That is what stops a
/// session vanishing from the list by having been continued once, after the
/// daemon that ran it is gone (ADR 0016 D2).
#[test]
fn a_known_session_keeps_its_own_id_and_is_still_a_row() {
    let mut rows = HistoryRows::default();
    let known = CorralSessionId::mint();
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    rows.replace(
        KnownProvider::Claude,
        Vec::new(),
        vec![(known, at_entry("k", at))],
        0,
    );

    let listed = rows.rows();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session, known, "its own id, nothing minted");
    assert_eq!(rows.last_active(known), Some(at));
    // Not continuable as a history row: it has a Session, so its continuation
    // is decided from the registry.
    assert_eq!(rows.row(known), None);
}

/// Recency is part of the pass that observed it. A store entry that is
/// deleted, or ages out of the enumeration window, stops being evidence that
/// the Session acted then — and `session.list` encodes this value straight
/// through, so a map only ever added to would keep showing it until the
/// daemon restarts.
#[test]
fn a_recency_a_later_pass_no_longer_sees_is_retracted() {
    let mut rows = HistoryRows::default();
    let known = CorralSessionId::mint();
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    rows.replace(
        KnownProvider::Claude,
        Vec::new(),
        vec![(known, at_entry("k", at))],
        0,
    );
    assert_eq!(rows.last_active(known), Some(at));

    rows.replace(KnownProvider::Claude, Vec::new(), Vec::new(), 0);

    assert_eq!(rows.last_active(known), None, "the store stopped saying so");
}

/// One Session can be held in more than one store, and the question
/// `session.list` asks is when it last acted — so a provider's pass replaces
/// its own answer without silencing the other's.
#[test]
fn recency_is_the_newest_across_providers() {
    let mut rows = HistoryRows::default();
    let known = CorralSessionId::mint();
    let older = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    let newer = SystemTime::UNIX_EPOCH + Duration::from_secs(9);
    rows.replace(
        KnownProvider::Claude,
        Vec::new(),
        vec![(known, at_entry("k", older))],
        0,
    );
    let mut elsewhere = at_entry("k", newer);
    elsewhere.provider = KnownProvider::Codex;
    rows.replace(
        KnownProvider::Codex,
        Vec::new(),
        vec![(known, elsewhere)],
        0,
    );
    assert_eq!(rows.last_active(known), Some(newer));

    rows.replace(KnownProvider::Codex, Vec::new(), Vec::new(), 0);

    assert_eq!(
        rows.last_active(known),
        Some(older),
        "the store that still says so is still heard"
    );
}

/// Retracting a provider takes back everything its store was the evidence
/// for — the rows and the recency alike — and touches no other provider's.
#[test]
fn retracting_a_provider_takes_back_its_rows_and_its_recency() {
    let mut rows = HistoryRows::default();
    let known = CorralSessionId::mint();
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    rows.replace(
        KnownProvider::Claude,
        vec![entry("a", 10)],
        vec![(known, at_entry("k", at))],
        0,
    );
    let mut codex = entry("t", 1);
    codex.provider = KnownProvider::Codex;
    let elsewhere = CorralSessionId::mint();
    let mut theirs = at_entry("u", at);
    theirs.provider = KnownProvider::Codex;
    rows.replace(
        KnownProvider::Codex,
        vec![codex],
        vec![(elsewhere, theirs)],
        0,
    );

    rows.retract(KnownProvider::Claude);

    assert_eq!(rows.last_active(known), None);
    assert_eq!(rows.last_active(elsewhere), Some(at));
    let listed = rows.rows();
    assert!(
        listed
            .iter()
            .all(|row| row.entry.provider == KnownProvider::Codex),
        "nothing Claude's store was the evidence for is left"
    );
    assert_eq!(listed.len(), 2, "and everything Codex's still is, is");
}

/// A pass resolves each entry against the registry one at a time and
/// publishes the lot at the end. A continuation that lands in between gives
/// one of those identities a Session and forgets its row — and the pass's
/// answer for it is from before that. Republished, it would mint a second id
/// for a provider session that now has one, and that row would be offered
/// for Continue: the spawn happens before the store refuses the duplicate,
/// so a stale snapshot could start a provider process nothing asked for
/// (ADR 0016 D2).
#[test]
fn a_pass_that_resolved_before_a_session_was_claimed_does_not_publish() {
    let mut rows = HistoryRows::default();
    rows.replace(KnownProvider::Claude, vec![entry("a", 10)], Vec::new(), 0);
    let listed = rows.rows()[0].session;

    // What a pass would have read before the continuation.
    let resolved_at = rows.generation();
    rows.forget(
        KnownProvider::Claude,
        &ExternalId::new("a").expect("usable"),
    );

    assert_eq!(
        rows.replace(
            KnownProvider::Claude,
            vec![entry("a", 5)],
            Vec::new(),
            resolved_at
        ),
        Published::Stale
    );
    assert!(
        rows.rows().is_empty(),
        "no second id for one provider session"
    );

    // The next pass reads the store as it now stands and is published.
    let now = rows.generation();
    assert_eq!(
        rows.replace(KnownProvider::Claude, vec![entry("a", 5)], Vec::new(), now),
        Published::Installed
    );
    assert_ne!(
        rows.rows()[0].session,
        listed,
        "a fresh identity, freshly read"
    );
}
