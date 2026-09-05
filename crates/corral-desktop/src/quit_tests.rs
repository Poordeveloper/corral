use super::*;

use std::time::{Duration, SystemTime};

use corral_client::sessions::Listing;
use corral_protocol::method::{AttentionCount, AttentionSummaryResult, SessionListResult};
use serde_json::{Value, json};

use crate::bridge::{Capabilities, Polled, Unanswered};

fn session(id: &str, origin: Option<&str>, execution_state: &str) -> Value {
    let mut row = json!({ "session_id": id, "title": "sh", "execution_state": execution_state });
    if let Some(origin) = origin {
        row["origin"] = Value::String(origin.to_owned());
    }
    row
}

fn list_of(sessions: Vec<Value>) -> SessionList {
    let mut list = SessionList::default();
    list.take(
        Ok(Polled {
            listing: Listing::of(SessionListResult { sessions }),
            summary: AttentionSummaryResult {
                needs_you: AttentionCount {
                    total: 0,
                    unacknowledged: 0,
                },
                ready: AttentionCount {
                    total: 0,
                    unacknowledged: 0,
                },
            },
            capabilities: Capabilities::default(),
        }),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1),
    );
    list
}

fn warning(list: &SessionList) -> Warning {
    match gate(list) {
        Gate::Warn(warning) => warning,
        Gate::Quit => panic!("expected a warning"),
    }
}

/// Only what the daemon calls managed counts, and running and unknown are
/// never one number.
#[test]
fn the_counts_come_from_origin_and_execution_state_kept_apart() {
    let list = list_of(vec![
        session("m-run-1", Some("managed"), "running"),
        session("m-run-2", Some("managed"), "running"),
        session("m-unknown", Some("managed"), "unknown"),
        session("m-newer-word", Some("managed"), "suspended"),
        session("m-exited", Some("managed"), "exited"),
        session("discovered", Some("discovered"), "running"),
        session("history", Some("history"), "unknown"),
        session("no-origin", None, "running"),
    ]);

    assert_eq!(
        continuing(list.rows()),
        Continuing {
            running: 2,
            unverified: 2,
        }
    );
}

#[test]
fn nothing_continuing_quits_without_a_word() {
    let list = list_of(vec![
        session("m-exited", Some("managed"), "exited"),
        session("discovered", Some("discovered"), "running"),
    ]);

    assert_eq!(gate(&list), Gate::Quit);
}

#[test]
fn running_sessions_are_said_to_continue() {
    let one = list_of(vec![session("a", Some("managed"), "running")]);
    assert_eq!(
        warning(&one),
        Warning {
            message: "1 session will continue running.".to_owned(),
            detail: "Corral will stop watching them for attention.",
        }
    );

    let two = list_of(vec![
        session("a", Some("managed"), "running"),
        session("b", Some("managed"), "running"),
    ]);
    assert_eq!(warning(&two).message, "2 sessions will continue running.");
}

/// Unknown is said as unknown: never "will continue", never "have stopped".
#[test]
fn unverified_sessions_keep_their_uncertainty() {
    let both = list_of(vec![
        session("a", Some("managed"), "running"),
        session("b", Some("managed"), "unknown"),
    ]);
    assert_eq!(
        warning(&both),
        Warning {
            message: "1 session is still running. Corral couldn't verify whether 1 other \
                      session it started has ended."
                .to_owned(),
            detail: "Corral will stop watching for attention.",
        }
    );

    let only_unknown = list_of(vec![
        session("b", Some("managed"), "unknown"),
        session("c", Some("managed"), "unknown"),
    ]);
    assert_eq!(
        warning(&only_unknown).message,
        "Corral couldn't verify whether 2 sessions it started have ended."
    );
}

/// Missing data is never zero: with no current answer the gate warns with
/// the uncertainty, and the person may still quit.
#[test]
fn an_unreachable_daemon_is_never_counted_as_nothing_running() {
    let mut list = list_of(vec![session("a", Some("managed"), "running")]);
    list.take(
        Err(Unanswered::Silent("gone".to_owned())),
        SystemTime::UNIX_EPOCH + Duration::from_secs(2),
    );
    assert_eq!(
        warning(&list),
        Warning {
            message: "Corral can't reach its service, so it can't tell whether sessions it \
                      started are still running."
                .to_owned(),
            detail: "Corral will stop watching for attention.",
        }
    );

    assert_eq!(
        warning(&SessionList::default()).detail,
        "Corral will stop watching for attention."
    );
}
