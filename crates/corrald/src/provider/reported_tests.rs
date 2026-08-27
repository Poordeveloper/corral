use std::time::{Duration, SystemTime};

use super::*;
use crate::provider::AgentFactKind;

fn fact(kind: AgentFactKind, seconds: u64) -> AgentFact {
    AgentFact {
        kind,
        observed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
    }
}

fn id(raw: &str) -> ExternalId {
    ExternalId::new(raw).expect("a usable external id")
}

/// A managed session says which agent it runs from the launch itself, before
/// any hook has fired: a row that stayed silent until the first event would
/// call the provider unknown when Corral is the one that started it.
#[test]
fn a_launched_session_is_known_by_its_provider_before_it_reports() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);

    let held = reported.get(session).expect("a launched session");
    assert_eq!(held.provider, KnownProvider::Claude);
    assert_eq!(held.external_id, None);
    assert_eq!(held.latest, None);
}

/// A resume is the same Session running again, so it keeps what it learned.
#[test]
fn relaunching_a_session_keeps_what_it_already_learned() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);
    reported.identified(session, KnownProvider::Claude, id("abc"));

    reported.launched(session, KnownProvider::Claude);

    assert_eq!(
        reported
            .get(session)
            .and_then(|held| held.external_id.clone()),
        Some(id("abc")),
    );
}

/// Superseded, never accumulated: a newer fact retires the older one, so an
/// `awaiting_input` is not still on a row after a turn started (ADR 0004 D7).
#[test]
fn a_newer_fact_retires_the_one_before_it() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::AwaitingInput, 10),
    );
    assert_eq!(
        reported
            .get(session)
            .and_then(|held| held.latest)
            .map(|held| held.kind),
        Some(AgentFactKind::AwaitingInput),
    );

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnStarted, 70),
    );
    let held = reported
        .get(session)
        .and_then(|held| held.latest)
        .expect("a fact");
    assert_eq!(held.kind, AgentFactKind::TurnStarted);
    assert_eq!(
        held.observed_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(70)
    );
}

/// Withdraw exactly the claim that became unsafe. The provider and the
/// reported facts stay known, and the conflicting id is never promoted into a
/// replacement (ADR 0004 D8, R2 Q3).
#[test]
fn withdrawing_an_identity_leaves_every_other_fact_standing() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);
    reported.identified(session, KnownProvider::Claude, id("first"));
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnEnded, 5),
    );

    reported.withdraw_identity(session);

    let held = reported.get(session).expect("a session");
    assert_eq!(held.external_id, None, "the claim is withdrawn");
    assert_eq!(
        held.provider,
        KnownProvider::Claude,
        "the product is still known"
    );
    assert_eq!(
        held.latest.map(|fact| fact.kind),
        Some(AgentFactKind::TurnEnded),
        "the facts it reported are still known",
    );
}

/// Withdrawal is not deletion, and a later report must not quietly restore the
/// claim by creating a fresh entry.
#[test]
fn a_fact_after_a_withdrawal_does_not_restore_the_identity() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.identified(session, KnownProvider::Claude, id("first"));
    reported.withdraw_identity(session);

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnStarted, 9),
    );

    assert_eq!(
        reported
            .get(session)
            .and_then(|held| held.external_id.clone()),
        None
    );
}

#[test]
fn a_session_nothing_launched_is_unknown() {
    let reported = ReportedSessions::new();
    assert!(reported.get(CorralSessionId::mint()).is_none());
}

#[test]
fn a_forgotten_session_is_unknown_again() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);
    reported.forget(session);
    assert!(reported.get(session).is_none());
}

/// Latest by observation, not by arrival. Each hook is delivered by its own
/// process over its own connection, so two events fired back to back can be
/// accepted in either order — and a row that went backwards would make
/// `latest` mean "most recently delivered", which is not what it says.
#[test]
fn an_out_of_order_delivery_does_not_move_a_row_backwards() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::AwaitingInput, 90),
    );
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnEnded, 30),
    );

    let held = reported
        .get(session)
        .and_then(|held| held.latest)
        .expect("a fact");
    assert_eq!(held.kind, AgentFactKind::AwaitingInput);
    assert_eq!(
        held.observed_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(90)
    );
}

/// Two facts stamped at the same instant are the ordinary case on a coarse
/// clock, and the later arrival is the better answer for them.
#[test]
fn a_fact_at_the_same_instant_supersedes() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnStarted, 42),
    );
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnEnded, 42),
    );

    assert_eq!(
        reported
            .get(session)
            .and_then(|held| held.latest)
            .map(|held| held.kind),
        Some(AgentFactKind::TurnEnded),
    );
}
