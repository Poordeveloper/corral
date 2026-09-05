use super::*;

use corral_client::sessions::Listing;
use corral_protocol::method::{AttentionCount, AttentionSummaryResult, SessionListResult};
use serde_json::{Value, json};

use crate::bridge::{Capabilities, Polled, Unanswered};

/// A row in the shape the daemon sends, with an attention claim entered at
/// `since` and one current item, acknowledged or not.
fn attention(id: &str, state: &str, since_ms: i64, acknowledged: bool) -> Value {
    json!({
        "session_id": id,
        "title": format!("title {id}"),
        "execution_state": "running",
        "attention": {
            "state": state,
            "since_unix_ms": since_ms,
            "items": [{
                "attention_item_id": format!("item-{id}"),
                "reason": if state == "ready" { "turn_complete" } else { "needs_input" },
                "since_unix_ms": since_ms,
                "acknowledged": acknowledged,
            }],
        },
    })
}

fn plain(id: &str, state: &str) -> Value {
    json!({ "session_id": id, "title": "sh", "execution_state": state })
}

fn count(total: u32, unacknowledged: u32) -> AttentionCount {
    AttentionCount {
        total,
        unacknowledged,
    }
}

fn polled(sessions: Vec<Value>, needs_you: AttentionCount, ready: AttentionCount) -> Polled {
    Polled {
        listing: Listing::of(SessionListResult { sessions }),
        summary: AttentionSummaryResult { needs_you, ready },
        capabilities: Capabilities::default(),
    }
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn current(projection: &TrayProjection) -> &Current {
    match projection {
        TrayProjection::Current(current) => current,
        TrayProjection::Unreachable { line } => panic!("unreachable: {line}"),
    }
}

#[test]
fn the_badge_is_the_unacknowledged_total_of_both_classes_bounded_for_the_menu_bar() {
    assert_eq!(Badge(0).text(), None);
    assert_eq!(Badge(1).text(), Some("1".to_owned()));
    assert_eq!(Badge(99).text(), Some("99".to_owned()));
    assert_eq!(Badge(100).text(), Some("99+".to_owned()));

    let mut list = SessionList::default();
    list.take(Ok(polled(vec![], count(3, 2), count(2, 1))), at(100));

    let projection = TrayProjection::of(&list, at(100));
    assert_eq!(current(&projection).badge, Badge(3));
    assert_eq!(projection.badge_text(), Some("3".to_owned()));
    assert_eq!(projection.header(), "Needs You 3 · Ready 2");
}

/// The header counts every row including the acknowledged ones, so rows and
/// header agree; the badge does not, and the marker says which row is why.
#[test]
fn an_acknowledged_row_stays_listed_without_the_marker_and_off_the_badge() {
    let mut list = SessionList::default();
    list.take(
        Ok(polled(
            vec![
                attention("a-1", "needs_you", 90_000, false),
                attention("b-2", "needs_you", 80_000, true),
            ],
            count(2, 1),
            count(0, 0),
        )),
        at(100),
    );

    let projection = TrayProjection::of(&list, at(100));
    let group = &current(&projection).needs_you;
    assert_eq!(group.total, 2);
    assert_eq!(group.rows.len(), 2);
    assert_eq!(current(&projection).badge, Badge(1));
    assert_eq!(group.rows[0].text(), "• title a-1 · Needs You · <1m");
    assert_eq!(
        group.rows[0].unacknowledged_item.as_deref(),
        Some("item-a-1")
    );
    assert_eq!(group.rows[1].text(), "title b-2 · Needs You · <1m");
    assert!(group.rows[1].acknowledged());
}

#[test]
fn rows_keep_the_daemons_order_and_the_rest_is_counted_not_hidden() {
    let sessions: Vec<Value> = (0..12)
        .map(|n| attention(&format!("s-{n:02}"), "needs_you", 0, false))
        .collect();
    let mut list = SessionList::default();
    list.take(Ok(polled(sessions, count(12, 12), count(0, 0))), at(1));

    let projection = TrayProjection::of(&list, at(1));
    let group = &current(&projection).needs_you;
    let ids: Vec<&str> = group
        .rows
        .iter()
        .map(|row| row.session_id.as_str())
        .collect();
    assert_eq!(ids.len(), ROWS_PER_GROUP);
    assert_eq!(ids[0], "s-00");
    assert_eq!(ids[9], "s-09");
    assert_eq!(group.overflow, 2);
    assert_eq!(group.overflow_line(), Some("… 2 more in Corral".to_owned()));
    assert_eq!(current(&projection).ready.overflow_line(), None);
}

#[test]
fn only_needs_you_and_ready_reach_the_tray() {
    let mut list = SessionList::default();
    list.take(
        Ok(polled(
            vec![
                attention("w", "working", 0, false),
                plain("u", "running"),
                plain("x", "exited"),
                attention("r", "ready", 0, false),
                attention("n", "needs_you", 0, false),
            ],
            count(1, 1),
            count(1, 1),
        )),
        at(1),
    );

    let projection = TrayProjection::of(&list, at(1));
    let current = current(&projection);
    assert_eq!(current.needs_you.rows.len(), 1);
    assert_eq!(current.needs_you.rows[0].session_id, "n");
    assert_eq!(current.needs_you.rows[0].state, MainState::NeedsYou);
    assert_eq!(current.ready.rows.len(), 1);
    assert_eq!(current.ready.rows[0].session_id, "r");
    assert_eq!(current.ready.rows[0].state, MainState::Ready);
}

/// A second of clock changes nothing; a minute changes the age, and only
/// then does the value — and the native menu with it — change.
#[test]
fn a_second_of_clock_does_not_change_the_projection_but_an_age_bucket_does() {
    let mut list = SessionList::default();
    list.take(
        Ok(polled(
            vec![attention("a-1", "ready", 100_000, false)],
            count(0, 0),
            count(1, 1),
        )),
        at(130),
    );

    let first = TrayProjection::of(&list, at(130));
    assert_eq!(TrayProjection::of(&list, at(131)), first);
    assert_eq!(TrayProjection::of(&list, at(159)), first);

    let later = TrayProjection::of(&list, at(160));
    assert_ne!(later, first);
    assert_eq!(current(&later).ready.rows[0].age.as_deref(), Some("1m"));
}

#[test]
fn ages_are_buckets_never_seconds() {
    assert_eq!(age_bucket(Duration::from_secs(0)), "<1m");
    assert_eq!(age_bucket(Duration::from_secs(59)), "<1m");
    assert_eq!(age_bucket(Duration::from_secs(60)), "1m");
    assert_eq!(age_bucket(Duration::from_secs(3_599)), "59m");
    assert_eq!(age_bucket(Duration::from_secs(3_600)), "1h");
    assert_eq!(age_bucket(Duration::from_secs(172_799)), "47h");
    assert_eq!(age_bucket(Duration::from_secs(172_800)), "2d");
}

/// Stale counts are never shown as current: the item goes quiet and the
/// menu says what the window's banner says.
#[test]
fn a_list_that_is_not_current_projects_as_unreachable() {
    let mut list = SessionList::default();
    assert_eq!(
        TrayProjection::of(&list, at(0)),
        TrayProjection::Unreachable {
            line: ASKING.to_owned()
        }
    );

    list.take(
        Ok(polled(
            vec![attention("a-1", "needs_you", 0, false)],
            count(1, 1),
            count(0, 0),
        )),
        at(10),
    );
    list.take(Err(Unanswered::Silent("gone".to_owned())), at(20));

    let projection = TrayProjection::of(&list, at(20));
    assert_eq!(projection.badge_text(), None);
    assert_eq!(projection.header(), "corrald did not answer: gone");
    assert!(matches!(projection, TrayProjection::Unreachable { .. }));
}

#[test]
fn a_menu_id_carries_the_action_and_the_session_identity() {
    for action in [
        TrayAction::OpenCorral,
        TrayAction::NewSession,
        TrayAction::Quit,
        TrayAction::More,
        TrayAction::OpenSession("0192-abc".to_owned()),
    ] {
        assert_eq!(TrayAction::from_menu_id(&action.menu_id()), Some(action));
    }
    assert_eq!(TrayAction::from_menu_id("header"), None);
    assert_eq!(TrayAction::from_menu_id("group:Needs You"), None);
    assert_eq!(TrayAction::from_menu_id("session:"), None);
    assert_eq!(TrayAction::from_menu_id("something-newer"), None);
}
