use super::*;

use serde_json::{Value, json};

fn session(id: &str) -> Value {
    json!({
        "session_id": id,
        "title": "sh",
        "execution_state": "running",
    })
}

#[test]
fn rows_keep_the_daemons_order() {
    let listing = Listing::of(SessionListResult {
        sessions: vec![session("b-2"), session("a-1"), session("c-3")],
    });

    let ids: Vec<&str> = listing
        .items
        .iter()
        .map(|item| item.session_id.as_str())
        .collect();
    assert_eq!(ids, ["b-2", "a-1", "c-3"]);
    assert_eq!(listing.unreadable, 0);
}

/// A newer daemon's row carries fields this build has no words for. They
/// are skipped inside the row, which still decodes; the client refuses
/// nothing it can read (`AGENTS.md` §Protocol).
#[test]
fn a_row_with_fields_this_build_does_not_know_still_decodes() {
    let mut newer = session("s1-rest");
    newer["from_the_future"] = json!({ "shape": "unknown" });
    newer["attention"] = json!({ "state": "meditating" });

    let listing = Listing::of(SessionListResult {
        sessions: vec![newer],
    });

    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.unreadable, 0);
    assert_eq!(listing.items[0].session_id, "s1-rest");
}

/// A row this build cannot read at all is counted, never dropped silently
/// and never guessed at, and the rows around it are unaffected.
#[test]
fn a_row_this_build_cannot_read_is_counted_rather_than_dropped() {
    let listing = Listing::of(SessionListResult {
        sessions: vec![
            session("s0-rest"),
            json!({ "not": "a session" }),
            json!("a string where a row should be"),
            session("s3-rest"),
        ],
    });

    assert_eq!(listing.unreadable, 2);
    let ids: Vec<&str> = listing
        .items
        .iter()
        .map(|item| item.session_id.as_str())
        .collect();
    assert_eq!(ids, ["s0-rest", "s3-rest"]);
}

#[test]
fn the_short_id_is_the_first_group_and_an_id_without_one_is_itself() {
    assert_eq!(short_id("0f9b6c1a-0000-0000-0000-000000000000"), "0f9b6c1a");
    assert_eq!(short_id("plain"), "plain");
    assert_eq!(short_id(""), "");
}
