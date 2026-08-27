//! End-to-end: the session list, on a terminal, against a real daemon.
//!
//! The loop PR4 exists to prove — see every session, open one, come back —
//! is only true of the whole chain, and the surface only exists on a pty
//! (`support::pty`).

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::time::Duration;

use serde_json::{Value, json};
use support::corpus;
use support::pty::Terminal;
use support::wire::{FakeBehaviour, RawClient, spawn_fake_daemon};
use support::{SETTLE, TestAccount, wait_until};

/// What every frame the list draws begins with: the cursor hidden, the screen
/// cleared. It is the list's own drawing and nobody else's — an attached
/// session's snapshot clears without hiding the cursor — which is what makes
/// "the frame after the takeover" something a test can point at instead of a
/// moment it has to time.
const FRAME: &str = "\x1b[?25l\x1b[H\x1b[2J";

/// Giving the terminal back. The list writes it exactly once, on the way out —
/// a takeover happens on the screen the list already took, because a session
/// that cleared and drew on the person's own screen would consume the
/// scrollback the alternate screen exists to protect.
const RELEASE: &str = "\x1b[?1049l";

/// Taking it. Written once on the way in, and again on the way back from every
/// takeover: a session replays its child's own bytes, and a child that leaves
/// the alternate screen on exit would otherwise leave the list drawing — and
/// clearing — the person's own.
const TAKE: &str = "\x1b[?1049h";

/// Turning mouse reporting off, which the list does wherever it claims the
/// terminal.
const NO_MOUSE: &str = "\x1b[?1000l";

const ROWS: u16 = 24;
const COLS: u16 = 80;

fn start_session(client: &mut RawClient, id: u64, argv: &[&str]) -> String {
    let started = client
        .request(
            id,
            "session.new",
            Some(json!({
                "command_id": format!("cmd-{id}"),
                "argv": argv,
                "rows": ROWS,
                "cols": COLS,
            })),
        )
        .expect("session.new answered");
    started
        .get("outcome")
        .and_then(|outcome| outcome.get("result"))
        .and_then(|result| result.get("session_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("not a started session: {started}"))
        .to_owned()
}

fn sessions(client: &mut RawClient, id: u64) -> usize {
    let listed = client
        .request(id, "session.list", None)
        .expect("session.list answered");
    listed
        .get("outcome")
        .and_then(|outcome| outcome.get("result"))
        .and_then(|result| result.get("sessions"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

/// The first whole frame the list drew after `from`.
///
/// Complete once the next one starts, and the list redraws every second, so
/// this waits for a fact rather than timing one.
fn frame_after(terminal: &Terminal, from: usize) -> String {
    let first = terminal.wait_for_after(from, FRAME);
    let next = terminal.wait_for_after(first, FRAME);
    terminal.between(first, next - FRAME.len())
}

/// The whole loop: see the session, take it over, come back to a list that is
/// current rather than the one left behind (grill Q1, Q4).
#[test]
fn the_list_opens_a_session_and_comes_back_to_a_current_one() {
    let account = TestAccount::new("tui-open");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    start_session(
        &mut client,
        1,
        &["/bin/sh", "-c", "printf 'in-the-session\\r\\n'; sleep 30"],
    );

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    terminal.wait_for("Running · Status unknown");

    let opened = terminal.typed(b"\r");
    // The session's own screen, replayed into this terminal: the takeover
    // happened, and it is the attachment that already existed.
    let painted = terminal.wait_for_after(opened, "in-the-session");

    assert!(
        !terminal.between(opened, painted).contains(RELEASE),
        "the takeover left the alternate screen, so the session cleared and \
         drew over the person's own"
    );

    // Started while the person was inside that session. The list they come
    // back to can only hold it if it asked again on the way back.
    start_session(&mut client, 2, &["/bin/sleep", "30"]);
    let detached = terminal.typed(b"\x1c");

    terminal.wait_for_after(detached, TAKE);
    // A session may have turned mouse reporting on and died with it on. A
    // terminal still reporting sends three coordinate bytes for every click,
    // and a click in the eighty-first column sends `q`.
    terminal.wait_for_after(detached, NO_MOUSE);
    let returned = frame_after(&terminal, detached);

    assert!(
        returned.contains("sleep"),
        "the list came back stale rather than current:\n{returned}"
    );
    assert!(
        returned.contains("Corral — 2 sessions"),
        "the list did not rebuild:\n{returned}"
    );

    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}

/// Open is refused before the keystroke, not after it. The row stays, and its
/// execution state keeps saying the process is running (grill Q7).
#[test]
fn a_session_whose_screen_cannot_be_served_refuses_to_be_opened() {
    let account = TestAccount::new("tui-no-screen");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let script = format!("cat '{}'; sleep 30", corpus::poisoning_input().display());
    start_session(&mut client, 1, &["/bin/sh", "-c", &script]);

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    let said = terminal.wait_for("Screen unavailable");

    let refused = terminal.typed(b"\r");
    terminal.wait_for_after(refused, "this session cannot be opened.");
    let still_listed = frame_after(&terminal, refused);

    assert!(
        still_listed.contains("Running · Status unknown"),
        "a screen Corral cannot serve was turned into a claim about the process:\n{still_listed}"
    );
    assert!(said > 0);

    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}

/// `new` is a prompt, a `session.new`, and straight into the session — the
/// same path `corral new` takes (grill Q1).
#[test]
fn a_command_typed_at_the_prompt_starts_a_session_and_opens_it() {
    let account = TestAccount::new("tui-new");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    terminal.wait_for("No sessions.");

    terminal.typed(b"n");
    terminal.wait_for("new session: ");
    terminal.typed(b"/bin/sleep 30\r");
    // The daemon is what says the session exists; the surface is inside it by
    // then, which is why the next keystroke detaches rather than navigates.
    let mut id = 1;
    wait_until(SETTLE, || {
        id += 1;
        sessions(&mut client, id) == 1
    });

    let detached = terminal.typed(b"\x1c");
    let returned = frame_after(&terminal, detached);

    assert!(
        returned.contains("sleep"),
        "the session the person started is not in the list they came back to:\n{returned}"
    );

    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}

/// A daemon that goes away is said so, and the list does not keep drawing what
/// it last held. It is a retry, not a dead surface: the next poll activates,
/// exactly as any other client would (grill Q4).
#[test]
fn a_lost_daemon_is_reported_and_the_list_recovers() {
    let account = TestAccount::new("tui-lost-daemon");
    let daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    start_session(&mut client, 1, &["/bin/sh", "-c", "sleep 30"]);

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    terminal.wait_for("Corral — 1 session");

    daemon.signal(rustix::process::Signal::KILL);
    let reported = terminal.wait_for("corrald did not answer");
    let disconnected = frame_after(&terminal, reported);

    assert!(
        !disconnected.contains("Running · Status unknown"),
        "the last answer was left on screen as though it were current:\n{disconnected}"
    );

    // The daemon it activates is a new one, which is running nothing — so the
    // list coming back is the list being right, not the old one returning.
    terminal.wait_for_after(reported, "No sessions.");

    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}

/// A second question is never queued behind an unanswered one.
///
/// A surface that polled in a task of its own would build a backlog against a
/// slow daemon and then ask about a list nobody is waiting for any more
/// (grill Q4). Only a daemon far slower than a real one can show the
/// difference, so this one is a stand-in.
#[test]
fn a_slow_answer_does_not_build_a_queue_of_questions() {
    let account = TestAccount::new("tui-slow-list");
    let daemon = spawn_fake_daemon(
        &account.socket(),
        FakeBehaviour::AnswerSlowly {
            delay: Duration::from_millis(1_500),
        },
    );

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    terminal.wait_for("No sessions.");
    // Long enough for several polls, and for a backlog to have formed behind
    // answers this slow if one could.
    wait_until(Duration::from_secs(8), || daemon.requests() >= 3);

    assert!(
        !daemon.overlapped(),
        "a question was sent before the last one was answered"
    );

    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}

/// What a person typed after the key that opened a session was typed for the
/// session, and gets there.
///
/// One `read` can carry both. A surface that stopped decoding at the key it
/// acted on and dropped the rest would lose exactly the character they meant
/// for their agent — the failure `LocalKeys` exists to prevent.
#[test]
fn what_was_typed_after_open_reaches_the_session() {
    let account = TestAccount::new("tui-typed-through");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // Echoes what it is given, so what arrived is what is drawn.
    start_session(&mut client, 1, &["/bin/cat"]);

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    terminal.wait_for("Running · Status unknown");

    // One write: the key that opens, and what was meant for what it opened.
    let opened = terminal.typed(b"\rxyzzy");

    terminal.wait_for_after(opened, "xyzzy");

    terminal.typed(b"\x1c");
    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}

/// The keyboard keeps working while a daemon does not answer.
///
/// Raw mode holds `Ctrl-C` and `Ctrl-\`, so a surface that stops reading keys
/// while it waits for an answer is one a person cannot leave from the terminal
/// they are sitting at. Only a daemon far slower than a real one can hold the
/// question open long enough to type into.
#[test]
fn the_keyboard_answers_while_a_daemon_does_not() {
    let account = TestAccount::new("tui-key-during-wait");
    let _daemon = spawn_fake_daemon(
        &account.socket(),
        FakeBehaviour::AnswerSlowly {
            delay: Duration::from_secs(30),
        },
    );

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    // Drawn before any answer: the first question is outstanding from here.
    terminal.wait_for("Asking corrald…");

    terminal.typed(b"q");

    // Sooner than `ANSWER`, deliberately: a surface that only reads the key
    // once its own patience with the daemon runs out has not read it.
    let left = terminal
        .exited_within(Duration::from_secs(2))
        .expect("the surface answered the key while the question was outstanding");
    assert!(left.success());
}

/// And the wait itself is bounded. A question with no answer is given up on
/// and said so, rather than leaving the last frame up forever.
#[test]
fn a_question_that_is_never_answered_is_given_up_on() {
    let account = TestAccount::new("tui-no-answer");
    let _daemon = spawn_fake_daemon(
        &account.socket(),
        FakeBehaviour::AnswerSlowly {
            delay: Duration::from_secs(30),
        },
    );

    let mut terminal = Terminal::spawn(account.corral_on_pty(&["tui"]), ROWS, COLS);
    let asking = terminal.wait_for("Asking corrald…");

    // Both halves of the claim in one line: the list stopped waiting, and it
    // says that is why.
    terminal.wait_for_after(asking, "corrald did not answer: nothing within");

    terminal.typed(b"q");
    assert!(terminal.wait_for_exit().success());
}
