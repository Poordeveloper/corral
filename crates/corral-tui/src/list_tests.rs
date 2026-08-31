use super::*;

use std::time::SystemTime;

use serde_json::{Value, json};

fn session(id: &str, execution_state: &str, terminal_access: Option<&str>) -> Value {
    let mut value = json!({
        "session_id": id,
        "title": "sh",
        "execution_state": execution_state,
    });
    if let Some(access) = terminal_access {
        value["terminal_access"] = json!(access);
    }
    value
}

fn answered(sessions: Vec<Value>) -> Result<Listed, Unanswered> {
    Ok(decode(SessionListResult { sessions }))
}

fn lost() -> Result<Listed, Unanswered> {
    Err(Unanswered::Silent("the daemon went away".to_owned()))
}

fn running(count: usize) -> Vec<Value> {
    (0..count)
        .map(|index| session(&format!("s{index}-rest"), "running", Some("available")))
        .collect()
}

/// The disconnected presentation. A list that keeps drawing its last answer
/// while the daemon is gone is showing a memory as current truth, which is the
/// one thing grill Q4 forbids it to do.
#[test]
fn a_daemon_that_cannot_be_read_empties_the_list_rather_than_freezing_it() {
    let mut list = SessionList::default();
    list.take(answered(running(2)));

    list.take(lost());

    assert!(list.rows.is_empty(), "a stale list was left on screen");
    assert!(
        list.unanswered
            .as_ref()
            .is_some_and(|unanswered| unanswered.line().contains("did not answer"))
    );
}

/// And it comes back on its own. The poll is the retry: a person who restarted
/// `corrald` does not restart this too.
#[test]
fn an_answer_after_a_loss_puts_the_list_back() {
    let mut list = SessionList::default();
    list.take(lost());

    list.take(answered(running(1)));

    assert_eq!(list.rows.len(), 1);
    assert!(list.unanswered.is_none());
}

/// The daemon orders by start time, so a session starting elsewhere moves
/// every row down. The cursor follows the session it was on, or a person is
/// one keystroke from opening something they did not choose.
#[test]
fn the_cursor_follows_the_session_it_was_on() {
    let mut list = SessionList::default();
    list.take(answered(vec![
        session("older-1", "running", Some("available")),
        session("oldest-2", "running", Some("available")),
    ]));
    list.selected = 1;

    list.take(answered(vec![
        session("newest-0", "running", Some("available")),
        session("older-1", "running", Some("available")),
        session("oldest-2", "running", Some("available")),
    ]));

    assert_eq!(list.rows[list.selected].session_id, "oldest-2");
}

/// Refused before the keystroke, not after it: the row stays, its execution
/// state is untouched, and the reason was already on screen (grill Q7).
#[test]
fn open_is_refused_when_the_screen_cannot_be_served() {
    let mut list = SessionList::default();
    list.take(answered(vec![session(
        "s0-rest",
        "running",
        Some("unavailable"),
    )]));

    let chosen = list.act(Key::Enter);

    assert!(chosen.is_none(), "a session with no screen was opened");
    assert_eq!(
        list.notice.as_deref(),
        Some("Screen unavailable: this session cannot be opened.")
    );
    assert_eq!(
        list.rows.len(),
        1,
        "the row was removed instead of being refused"
    );
}

/// Unknown is not a refusal. A daemon that did not send the field, or sent a
/// word this build does not know, leaves Open on offer — whatever comes back
/// is the answer (`AGENTS.md` §Protocol).
#[test]
fn open_is_offered_when_terminal_access_is_unknown() {
    for access in [None, Some("degraded")] {
        let mut list = SessionList::default();
        list.take(answered(vec![session("s0-rest", "running", access)]));

        let chosen = list.act(Key::Enter);

        assert!(
            matches!(chosen, Some(Chosen::Open(ref id)) if id == "s0-rest"),
            "{access:?} disabled an action on a value nothing understood"
        );
    }
}

fn typed(line: &[u8]) -> Option<Chosen> {
    let mut list = SessionList::default();
    list.act(Key::Typed('n'));
    for key in crate::keys::decode(line) {
        list.act(key);
    }
    list.act(Key::Enter)
}

/// The separator is what tells the two namespaces apart, and it is the same
/// separator the command line uses (grill Q6).
#[test]
fn a_command_typed_after_the_separator_becomes_the_program_and_its_arguments() {
    match typed(b"-- /bin/sh -c  sleep") {
        Some(Chosen::New(crate::launch::Requested::Command(argv))) => {
            assert_eq!(argv, ["/bin/sh", "-c", "sleep"]);
        }
        other => panic!("{}", other.map_or("nothing", |_| "something else")),
    }
}

/// A pasted command waits for the person to confirm it.
///
/// The whole path, from the bytes a terminal delivers to what the list does
/// with them: a clipboard entry ending in a newline is the ordinary shape, and
/// starting a session the moment one arrives would be an action performed
/// because of what was on a clipboard rather than because anybody asked.
#[test]
fn a_pasted_command_does_not_start_until_enter() {
    let mut list = SessionList::default();
    list.act(Key::Typed('n'));

    for key in crate::keys::decode(b"\x1b[200~-- /bin/sh -c sleep\r\x1b[201~") {
        assert!(
            list.act(key).is_none(),
            "a paste started something: {key:?}"
        );
    }

    // And the person's own Enter runs exactly what they pasted.
    match list.act(Key::Enter) {
        Some(Chosen::New(crate::launch::Requested::Command(argv))) => {
            assert_eq!(argv, ["/bin/sh", "-c", "sleep"]);
        }
        other => panic!("{}", other.map_or("nothing", |_| "something else")),
    }
}

/// On the list itself a paste is inert. `q` quits, `n` opens the prompt and
/// `c` continues a session; none of them may happen because a clipboard held
/// the letter.
#[test]
fn a_paste_on_the_list_performs_nothing() {
    let mut list = SessionList::default();

    for key in crate::keys::decode(b"\x1b[200~qnc\r\x1b[201~") {
        assert!(
            list.act(key).is_none(),
            "a paste acted on the list: {key:?}"
        );
    }

    assert!(
        matches!(list.act(Key::Typed('q')), Some(Chosen::Quit)),
        "the list still quits when the person types it",
    );
}

#[test]
fn an_agent_typed_at_the_prompt_becomes_a_provider_request() {
    match typed(b"claude") {
        Some(Chosen::New(crate::launch::Requested::Provider { name, args })) => {
            assert_eq!(name, "claude");
            assert!(args.is_empty());
        }
        other => panic!("{}", other.map_or("nothing", |_| "something else")),
    }
}

/// An agent's own arguments pass through, with or without the separator a
/// person may or may not type.
#[test]
fn an_agents_own_arguments_pass_through() {
    for line in [&b"claude --model opus"[..], &b"claude -- --model opus"[..]] {
        match typed(line) {
            Some(Chosen::New(crate::launch::Requested::Provider { name, args })) => {
                assert_eq!(name, "claude");
                assert_eq!(args, ["--model", "opus"]);
            }
            other => panic!(
                "{:?}: {}",
                std::str::from_utf8(line),
                other.map_or("nothing", |_| "something else")
            ),
        }
    }
}

/// Nothing is guessed. A name this surface does not recognise is still sent as
/// an agent, and the daemon — the one owner of the provider list — refuses it
/// by name.
#[test]
fn an_unrecognised_first_word_is_still_an_agent_request() {
    match typed(b"bash") {
        Some(Chosen::New(crate::launch::Requested::Provider { name, .. })) => {
            assert_eq!(name, "bash");
        }
        other => panic!("{}", other.map_or("nothing", |_| "something else")),
    }
}

/// A separator with nothing after it names neither an agent nor a command.
#[test]
fn a_separator_with_no_command_starts_nothing() {
    assert!(typed(b"--").is_none());
}

#[test]
fn escape_abandons_the_prompt_without_starting_anything() {
    let mut list = SessionList::default();
    list.act(Key::Typed('n'));
    list.act(Key::Typed('x'));

    let chosen = list.act(Key::Escape);

    assert!(chosen.is_none());
    assert_eq!(list.typing, None);
}

#[test]
fn an_empty_command_starts_nothing() {
    let mut list = SessionList::default();
    list.act(Key::Typed('n'));

    let chosen = list.act(Key::Enter);

    assert!(chosen.is_none());
}

/// While a command is being typed, the letters that are keys everywhere else
/// are letters.
#[test]
fn the_prompt_takes_the_keys_the_list_would_have_acted_on() {
    let mut list = SessionList::default();
    list.act(Key::Typed('n'));

    list.act(Key::Typed('q'));
    list.act(Key::Typed('n'));

    assert_eq!(list.typing.as_deref(), Some("qn"));
}

#[test]
fn backspace_removes_a_whole_character() {
    let mut list = SessionList::default();
    list.act(Key::Typed('n'));
    for key in crate::keys::decode("aé".as_bytes()) {
        list.act(key);
    }

    list.act(Key::Backspace);

    assert_eq!(list.typing.as_deref(), Some("a"));
}

#[test]
fn the_window_scrolls_only_as_far_as_the_selection_needs() {
    let mut list = SessionList::default();
    list.take(answered(running(5)));

    // Six lines holds three two-line rows.
    assert_eq!(list.window(6), 0..3);

    list.selected = 3;
    assert_eq!(list.window(6), 1..4);

    list.selected = 0;
    assert_eq!(list.window(6), 0..3);
}

/// A row with a capability line is taller, and the window has to know it or
/// the last row it admits runs off the bottom of the screen.
#[test]
fn a_row_that_says_more_takes_more_room() {
    let mut list = SessionList::default();
    list.take(answered(vec![
        session("s0-rest", "running", Some("unavailable")),
        session("s1-rest", "running", Some("available")),
    ]));

    assert_eq!(list.rows[0].height(), 3);
    assert_eq!(list.rows[1].height(), 2);
    assert_eq!(list.window(4), 0..1, "a three-line row was fitted into two");
}

/// A daemon newer than this build may describe a session in a shape this build
/// cannot read. Counting them is better than dropping them silently.
#[test]
fn a_session_this_build_cannot_render_is_counted_rather_than_dropped() {
    let mut list = SessionList::default();

    list.take(answered(vec![
        session("s0-rest", "running", Some("available")),
        json!({"session_id": "s1-rest"}),
    ]));

    assert_eq!(list.rows.len(), 1);
    assert_eq!(list.unrenderable, 1);
    assert_eq!(
        list.notice, None,
        "a fact about the list was written where an answer to the person goes"
    );
}

/// What the poll found and what the person's last action produced are two
/// different things on the screen. A poll that wrote into the second would
/// replace the answer to a keystroke a second after they read it — and would
/// leave it there once the fact stopped being true.
#[test]
fn a_poll_neither_overwrites_nor_outlives_what_an_action_said() {
    let mut list = SessionList::default();
    list.take(answered(vec![session(
        "s0-rest",
        "running",
        Some("unavailable"),
    )]));
    list.act(Key::Enter);
    let said = list.notice.clone().expect("the refusal is on screen");

    list.take(answered(vec![
        session("s0-rest", "running", Some("unavailable")),
        json!({"session_id": "s1-rest"}),
    ]));
    assert_eq!(
        list.notice.as_ref(),
        Some(&said),
        "the poll took the screen"
    );

    list.take(answered(vec![session(
        "s0-rest",
        "running",
        Some("unavailable"),
    )]));
    assert_eq!(
        list.unrenderable, 0,
        "a fact outlived the answer that held it"
    );
}

/// Navigation stops at the ends rather than wrapping: a list that jumps from
/// the last session to the first is one a person opens the wrong thing from.
#[test]
fn the_cursor_stops_at_the_ends() {
    let mut list = SessionList::default();
    list.take(answered(running(2)));

    list.act(Key::Up);
    assert_eq!(list.selected, 0);

    list.act(Key::Down);
    list.act(Key::Down);
    assert_eq!(list.selected, 1);
}

#[test]
fn a_row_says_what_the_projection_says() {
    let item: SessionListItem =
        serde_json::from_value(session("s0-rest", "running", Some("unavailable"))).expect("decode");

    let lines = Row::of(&item, SystemTime::now()).lines;

    assert_eq!(
        lines,
        vec![
            "s0  sh".to_owned(),
            "Running · Status unknown".to_owned(),
            "Screen unavailable".to_owned(),
        ]
    );
}

/// The heading counts what the daemon reported and nothing else. A list that
/// says "0 sessions" over a body saying the daemon is gone is contradicting
/// itself in the same frame.
#[test]
fn the_heading_counts_nothing_it_has_not_been_told() {
    let mut list = SessionList::default();
    assert_eq!(heading(&list), "Corral");

    list.take(answered(running(2)));
    assert_eq!(heading(&list), "Corral — 2 sessions");

    list.take(lost());
    assert_eq!(heading(&list), "Corral");

    // Told about three, able to draw two. The heading counts what it was told,
    // or it contradicts the line under the rows saying there is one more.
    list.take(answered(vec![
        session("s0-rest", "running", Some("available")),
        session("s1-rest", "running", Some("available")),
        json!({"session_id": "s2-rest"}),
    ]));
    assert_eq!(heading(&list), "Corral — 3 sessions");
}

/// A refusal is about the row the person was on, so moving off it takes the
/// message with them.
#[test]
fn moving_the_cursor_clears_what_the_last_action_said() {
    let mut list = SessionList::default();
    list.take(answered(vec![
        session("s0-rest", "running", Some("unavailable")),
        session("s1-rest", "running", Some("available")),
    ]));
    list.act(Key::Enter);
    assert!(list.notice.is_some());

    list.act(Key::Down);

    assert_eq!(list.notice, None);
}

/// One row, rendered once. The screen and the CLI both draw what `lines` says,
/// so neither can start saying something the other does not (grill Q2).
#[test]
fn the_frame_shows_the_lines_the_row_says_it_has() {
    let terminal = Geometry { rows: 24, cols: 80 };
    let mut list = SessionList::default();
    list.take(answered(vec![session(
        "s0-rest",
        "running",
        Some("unavailable"),
    )]));
    let row = &list.rows[0];
    let mut frame = Frame::new(terminal);

    draw_row(&mut frame, row, false, terminal);

    let drawn = frame.text().into_owned();
    for line in &row.lines {
        assert!(drawn.contains(line), "{line:?} was not drawn:\n{drawn:?}");
    }
}

/// A session that goes away does not send the cursor to the top of the list —
/// which, ordered newest first, is whatever started most recently and is one
/// keystroke from being opened.
#[test]
fn a_vanished_selection_leaves_the_cursor_where_it_was() {
    let mut list = SessionList::default();
    list.take(answered(running(4)));
    list.selected = 2;

    // The session it was on is gone, and a newer one is now at the top.
    list.take(answered(vec![
        session("newest-0", "running", Some("available")),
        session("s0-rest", "running", Some("available")),
        session("s1-rest", "running", Some("available")),
        session("s3-rest", "running", Some("available")),
    ]));

    assert_eq!(list.rows[list.selected].session_id, "s1-rest");
}

/// Room left over above the window is spent. A window that only ever moved
/// down leaves the rows before it unreachable on a screen with space for them
/// — after the terminal grew, or after the rows below the cursor went away.
#[test]
fn a_window_with_room_to_spare_shows_what_is_above_it() {
    let mut list = SessionList::default();
    list.take(answered(running(10)));

    // Six lines holds three two-line rows, and the cursor at the bottom pushes
    // the window down to them.
    list.selected = 9;
    assert_eq!(list.window(6), 7..10);

    // The same list on a screen with room for all of it.
    assert_eq!(list.window(20), 0..10);
}

/// The same, when the rows below the cursor are the ones that went away.
#[test]
fn a_window_left_past_the_end_comes_back_to_what_is_there() {
    let mut list = SessionList::default();
    list.take(answered(running(10)));
    list.selected = 9;
    list.window(6);

    list.take(answered(running(2)));

    assert_eq!(
        list.window(6),
        0..2,
        "the window stayed where the rows were"
    );
}

/// Zero is a count too, and the body already says it. "Corral — 0 sessions"
/// over "No sessions." is the same frame saying it twice.
#[test]
fn an_answer_with_nothing_in_it_is_not_counted_in_the_heading() {
    let mut list = SessionList::default();

    list.take(answered(vec![]));

    assert_eq!(heading(&list), "Corral");
}

/// Every precondition of a continuation the daemon owns stays the daemon's, so
/// the keystroke asks rather than judges: a surface that pre-refused a
/// contested identity or an unverifiable ending would be a second owner of a
/// rule that fails closed, and its answer is the one a person would see.
#[test]
fn continuing_a_row_asks_the_daemon_about_it() {
    let mut list = SessionList::default();
    list.take(answered(vec![session("s0-exited", "exited", None)]));

    let chosen = list.act(Key::Typed('c'));

    assert!(
        matches!(chosen, Some(Chosen::Continue(ref id)) if id == "s0-exited"),
        "c on a row asks for that row",
    );
}

/// The live case is the exception, and it is the one this surface already
/// knows: the row is displaying the runtime fact that answers it. Asking
/// anyway would hand the terminal over and take it back for a guaranteed
/// refusal — an alternate-screen teardown and rebuild a person watches for
/// nothing.
#[test]
fn continuing_a_running_row_is_refused_before_the_terminal_is_handed_over() {
    let mut list = SessionList::default();
    list.take(answered(vec![session("s0-running", "running", None)]));

    let chosen = list.act(Key::Typed('c'));

    assert!(chosen.is_none(), "a live row was handed to the daemon");
    let notice = list.notice.clone().expect("a reason on screen");
    assert!(notice.contains("cannot be continued"), "{notice}");
}

/// An empty list has no action, and inventing a message for it would be noise
/// on the one screen that already says "No sessions."
#[test]
fn continuing_with_nothing_selected_does_nothing() {
    let mut list = SessionList::default();
    list.take(answered(Vec::new()));

    assert!(list.act(Key::Typed('c')).is_none());
}

/// The footer is the whole key map: a person can see everything this surface
/// does without being told about it anywhere else.
#[test]
fn the_footer_names_every_action_the_list_has() {
    for action in ["open", "new", "continue", "quit"] {
        assert!(FOOTER.contains(action), "{FOOTER} omits {action}");
    }
}
