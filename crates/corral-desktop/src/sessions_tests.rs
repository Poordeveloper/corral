use super::*;

use std::time::Duration;

use corral_client::sessions::Listing;
use corral_protocol::method::{AttentionCount, SessionListResult};
use serde_json::{Value, json};

fn session(id: &str, state: &str) -> Value {
    json!({
        "session_id": id,
        "title": "sh",
        "execution_state": state,
    })
}

fn polled(sessions: Vec<Value>, needs_you: u32) -> Result<Polled, Unanswered> {
    Ok(Polled {
        listing: Listing::of(SessionListResult { sessions }),
        summary: AttentionSummaryResult {
            needs_you: AttentionCount {
                total: needs_you,
                unacknowledged: needs_you,
            },
            ready: AttentionCount {
                total: 0,
                unacknowledged: 0,
            },
        },
        capabilities: Capabilities {
            managed_sessions: true,
            ..Capabilities::default()
        },
    })
}

fn lost() -> Result<Polled, Unanswered> {
    Err(Unanswered::Silent("the daemon went away".to_owned()))
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

#[test]
fn nothing_is_claimed_before_the_daemon_answers() {
    let list = SessionList::default();

    assert!(!list.is_current());
    assert_eq!(list.heading(), "Corral");
    assert_eq!(list.empty_line(), Some("Asking corrald…"));
    assert_eq!(list.banner(at(0)), None);
}

#[test]
fn an_answer_publishes_rows_counts_and_capabilities_together() {
    let mut list = SessionList::default();

    list.take(
        polled(vec![session("a-1", "running"), session("b-2", "exited")], 1),
        at(10),
    );

    assert!(list.is_current());
    assert_eq!(list.rows().len(), 2);
    assert_eq!(list.heading(), "Corral — 2 sessions · Needs You 1");
    assert!(list.capabilities().managed_sessions);
    assert_eq!(list.empty_line(), None);
}

/// The disconnected presentation: the last answer stays as history with its
/// age, nothing about it is current, and the banner says both.
#[test]
fn a_lost_daemon_keeps_the_last_answer_as_history_and_offers_nothing_on_it() {
    let mut list = SessionList::default();
    list.take(polled(vec![session("a-1", "running")], 0), at(10));

    list.take(lost(), at(25));

    assert!(!list.is_current());
    assert_eq!(list.rows().len(), 1, "the last answer was thrown away");
    assert_eq!(
        list.banner(at(25)).as_deref(),
        Some("corrald did not answer: the daemon went away Showing what it last said, 15s ago.")
    );

    list.take(polled(vec![session("a-1", "running")], 0), at(30));
    assert!(list.is_current());
    assert_eq!(list.banner(at(30)), None);
}

#[test]
fn the_selection_follows_the_session_across_a_reorder() {
    let mut list = SessionList::default();
    list.take(
        polled(
            vec![session("a-1", "running"), session("b-2", "running")],
            0,
        ),
        at(1),
    );
    list.select("b-2");

    list.take(
        polled(
            vec![session("b-2", "running"), session("a-1", "running")],
            0,
        ),
        at(2),
    );

    assert_eq!(
        list.selected().map(|row| row.session_id.as_str()),
        Some("b-2")
    );
}

#[test]
fn a_vanished_selection_moves_to_the_row_that_took_its_place() {
    let mut list = SessionList::default();
    list.take(
        polled(
            vec![
                session("a-1", "running"),
                session("b-2", "running"),
                session("c-3", "running"),
            ],
            0,
        ),
        at(1),
    );
    list.select("c-3");

    list.take(
        polled(
            vec![session("a-1", "running"), session("b-2", "running")],
            0,
        ),
        at(2),
    );

    assert_eq!(
        list.selected().map(|row| row.session_id.as_str()),
        Some("b-2")
    );
}

#[test]
fn moving_the_selection_stops_at_the_ends_and_starts_at_the_top() {
    let mut list = SessionList::default();
    list.move_selection(1);
    assert!(list.selected().is_none(), "nothing to select");

    list.take(
        polled(
            vec![session("a-1", "running"), session("b-2", "running")],
            0,
        ),
        at(1),
    );
    list.move_selection(1);
    assert_eq!(
        list.selected().map(|row| row.session_id.as_str()),
        Some("a-1")
    );
    list.move_selection(5);
    assert_eq!(
        list.selected().map(|row| row.session_id.as_str()),
        Some("b-2")
    );
    list.move_selection(-9);
    assert_eq!(
        list.selected().map(|row| row.session_id.as_str()),
        Some("a-1")
    );
}

/// A row this build cannot read is counted in the heading and told about,
/// never dropped silently or guessed at.
#[test]
fn a_row_this_build_cannot_read_is_counted_rather_than_dropped() {
    let mut list = SessionList::default();

    list.take(
        polled(
            vec![session("a-1", "running"), json!({"not": "a session"})],
            0,
        ),
        at(1),
    );

    assert_eq!(list.rows().len(), 1);
    assert_eq!(list.unreadable(), 1);
    assert_eq!(list.heading(), "Corral — 2 sessions");
}

#[test]
fn a_row_says_what_the_projection_says() {
    let mut list = SessionList::default();
    list.take(polled(vec![session("a-1", "exited")], 0), at(1));

    let row = &list.rows()[0];
    assert_eq!(row.presentation.state_line(), "Exited");
    assert_eq!(row.title, "sh");
}
