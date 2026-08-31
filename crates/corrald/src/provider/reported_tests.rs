use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::provider::AgentFactKind;

fn fact(kind: AgentFactKind, seconds: u64) -> AgentFact {
    AgentFact {
        kind,
        observed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
    }
}

/// The monotonic instant the endpoint would have taken for a fact stamped
/// `seconds`. A test cannot name an `Instant`, so they are offsets from one
/// taken here — which is all supersession needs, since it only ever compares
/// arrivals with each other.
fn arrival(seconds: u64) -> Instant {
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *BASE.get_or_init(Instant::now) + Duration::from_secs(seconds)
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
    reported.identified(session, KnownProvider::Claude, RunId::mint(), id("abc"));

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
        arrival(10),
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
        arrival(70),
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
    reported.identified(session, KnownProvider::Claude, RunId::mint(), id("first"));
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnEnded, 5),
        arrival(5),
    );

    reported.contested(session);

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
    reported.identified(session, KnownProvider::Claude, RunId::mint(), id("first"));
    reported.contested(session);

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnStarted, 9),
        arrival(9),
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

/// Latest by arrival at the endpoint, not by the order ingestion reached
/// them. Each hook is delivered by its own process over its own connection and
/// is stamped as it lands, so the queue can hand them over in either order —
/// and a row that went backwards would make `latest` mean "most recently
/// interpreted", which is not what it says.
#[test]
fn an_out_of_order_delivery_does_not_move_a_row_backwards() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::AwaitingInput, 90),
        arrival(90),
    );
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnEnded, 30),
        arrival(30),
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

/// Two facts that arrived at the same instant are the ordinary case on a
/// coarse clock, and the later of them is the better answer.
#[test]
fn a_fact_at_the_same_instant_supersedes() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnStarted, 42),
        arrival(42),
    );
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnEnded, 42),
        arrival(42),
    );

    assert_eq!(
        reported
            .get(session)
            .and_then(|held| held.latest)
            .map(|held| held.kind),
        Some(AgentFactKind::TurnEnded),
    );
}

/// A wall clock that steps backwards must not freeze a row.
///
/// The stamp a fact carries is what a surface renders an age from, and NTP or
/// a person can move it. If supersession read it, every fact after the step
/// would look older than the one on screen and be discarded — leaving a row
/// asserting a turn that has since ended, for as long as the clock took to
/// catch up.
#[test]
fn a_clock_that_steps_backwards_does_not_freeze_a_row() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);

    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::AwaitingInput, 900),
        arrival(10),
    );
    // Later, on the only clock that cannot be stepped, and stamped earlier.
    reported.reported(
        session,
        KnownProvider::Claude,
        fact(AgentFactKind::TurnStarted, 100),
        arrival(11),
    );

    let held = reported
        .get(session)
        .and_then(|held| held.latest)
        .expect("a fact");
    assert_eq!(held.kind, AgentFactKind::TurnStarted);
    assert_eq!(
        held.observed_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(100),
        "the stamp is still the fact's own, however it compares",
    );
}

/// A refused identity settles only itself: the agent can mint a fresh one
/// (`/clear` does), and a report of one must still reach the store.
#[test]
fn a_refused_identity_settles_only_itself() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);

    reported.identity_claimed_elsewhere(session, id("foreign"));

    assert!(reported.identity_closed(session, &id("foreign")));
    assert!(!reported.identity_closed(session, &id("minted-after")));
}

/// A contest closes the question whole, and a refusal arriving after it does
/// not weaken that to one id.
#[test]
fn a_contest_is_not_weakened_to_one_identity() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    reported.launched(session, KnownProvider::Claude);
    reported.contested(session);

    reported.identity_claimed_elsewhere(session, id("foreign"));

    assert!(reported.identity_closed(session, &id("foreign")));
    assert!(reported.identity_closed(session, &id("any-other")));
}

/// What a durable confirmation is written on: the same identity, observed in a
/// Run that had not observed it. A provider that reports no session start has
/// no other way to say "again" (ADR 0009 D3).
#[test]
fn an_identity_is_observed_per_run_and_not_once_for_all_of_them() {
    let mut reported = ReportedSessions::new();
    let session = CorralSessionId::mint();
    let first = RunId::mint();
    let second = RunId::mint();
    reported.launched(session, KnownProvider::Codex);
    reported.identified(session, KnownProvider::Codex, first, id("abc"));

    assert!(reported.identity_observed_in(session, first, &id("abc")));
    // A later Run of the same Session has not observed it yet, which is what
    // makes its first report worth recording.
    assert!(!reported.identity_observed_in(session, second, &id("abc")));
    // Nor has this Run observed an identity it was never told about.
    assert!(!reported.identity_observed_in(session, first, &id("xyz")));

    reported.identified(session, KnownProvider::Codex, second, id("abc"));
    assert!(reported.identity_observed_in(session, second, &id("abc")));

    // A contest withdraws the claim, so there is nothing left to have been
    // observed — a report of any id after it deserves the store's answer.
    reported.contested(session);
    assert!(!reported.identity_observed_in(session, second, &id("abc")));
}
