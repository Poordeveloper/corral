//! End-to-end: a person starts a session, sees it, attaches to its terminal,
//! and leaves it running.
//!
//! These drive real daemons over a real socket, because what they prove — that
//! output reaches a screen the daemon owns, that detaching does not kill work,
//! that a token opens exactly one channel — is only true of the whole chain.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Instant;

use serde_json::{Value, json};
use support::wire::{RawClient, error_code};
use support::{SETTLE, TestAccount, run, stdout};

/// The frame header the terminal channel uses: kind, epoch, sequence, length.
const HEADER_BYTES: usize = 1 + 8 + 8 + 4;

fn start_session(client: &mut RawClient, id: u64, argv: &[&str]) -> Value {
    client
        .request(
            id,
            "session.new",
            Some(json!({ "argv": argv, "rows": 24, "cols": 80 })),
        )
        .expect("session.new answered")
}

fn result(frame: &Value) -> &Value {
    frame
        .get("outcome")
        .and_then(|outcome| outcome.get("result"))
        .unwrap_or_else(|| panic!("not a result: {frame}"))
}

fn session_id(frame: &Value) -> String {
    result(frame)
        .get("session_id")
        .and_then(Value::as_str)
        .expect("a session id")
        .to_owned()
}

/// A terminal channel: the halves of a connection that left JSON framing.
struct Channel {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

/// Read one terminal frame, waiting up to the settle budget for it.
fn read_frame(channel: &mut Channel) -> Option<(u8, Vec<u8>)> {
    channel
        .reader
        .get_ref()
        .set_read_timeout(Some(SETTLE))
        .expect("a read deadline");
    let stream = &mut channel.reader;
    let mut header = [0_u8; HEADER_BYTES];
    stream.read_exact(&mut header).ok()?;
    let length = u32::from_be_bytes([header[17], header[18], header[19], header[20]]) as usize;
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).ok()?;
    Some((header[0], payload))
}

/// Open a terminal data channel by redeeming a token.
fn open_channel(account: &TestAccount, token: &str) -> Channel {
    let mut client = RawClient::connect(&account.socket());
    let answer = client.say_hello_with_role(token);
    assert!(
        answer
            .get("outcome")
            .and_then(|outcome| outcome.get("result"))
            .is_some(),
        "the terminal hello was refused: {answer}"
    );
    let (writer, reader) = client.into_parts();
    Channel { writer, reader }
}

fn attach_token(client: &mut RawClient, id: u64, session: &str) -> Value {
    client
        .request(
            id,
            "terminal.attach",
            Some(json!({ "session_id": session })),
        )
        .expect("terminal.attach answered")
}

#[test]
fn a_new_session_appears_in_the_list_and_names_its_program() {
    let account = TestAccount::new("new-session-listed");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let started = start_session(&mut client, 1, &["/bin/sh", "-c", "sleep 30"]);
    let session = session_id(&started);

    let listed = client
        .request(2, "session.list", None)
        .expect("session.list answered");
    let sessions = result(&listed)
        .get("sessions")
        .and_then(Value::as_array)
        .expect("a session array");

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].get("session_id").and_then(Value::as_str),
        Some(session.as_str())
    );
    assert_eq!(
        sessions[0].get("title").and_then(Value::as_str),
        Some("sh"),
        "the title is the program, not its arguments"
    );
    assert_eq!(
        sessions[0].get("execution_state").and_then(Value::as_str),
        Some("running")
    );
}

/// A command that never exec'd is refused, and leaves no session behind: a Run
/// that never started must not be recorded as one that did.
#[test]
fn a_command_that_cannot_start_leaves_no_session() {
    let account = TestAccount::new("failed-exec");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let refused = start_session(&mut client, 1, &["/definitely/not/here"]);

    assert_eq!(error_code(&refused), Some("invalid_params"), "{refused}");

    let listed = client
        .request(2, "session.list", None)
        .expect("session.list answered");
    assert!(
        result(&listed)
            .get("sessions")
            .and_then(Value::as_array)
            .expect("a session array")
            .is_empty(),
        "a session was recorded for a command that never ran"
    );
}

/// The whole chain: a child writes, the daemon's screen holds it, and an
/// attaching client is sent it in a snapshot.
#[test]
fn attaching_delivers_the_screen_the_daemon_holds() {
    let account = TestAccount::new("attach-snapshot");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let started = start_session(
        &mut client,
        1,
        &["/bin/sh", "-c", "printf 'CORRAL-E2E-MARKER'; sleep 30"],
    );
    let session = session_id(&started);

    // The child's output has to reach the daemon's screen before a snapshot
    // can carry it; poll rather than sleep a fixed span.
    let deadline = Instant::now() + SETTLE;
    let mut seen = false;
    while Instant::now() < deadline && !seen {
        let granted = attach_token(&mut client, 2, &session);
        let token = result(&granted)
            .get("attach_token")
            .and_then(Value::as_str)
            .expect("a token")
            .to_owned();

        let mut channel = open_channel(&account, &token);
        if let Some((kind, payload)) = read_frame(&mut channel) {
            assert_eq!(kind, 1, "the first frame on a channel is a snapshot");
            seen = String::from_utf8_lossy(&payload).contains("CORRAL-E2E-MARKER");
        }
    }

    assert!(seen, "the child's output never reached an attaching client");
}

/// A token opens exactly one channel. Redemption consumes it, so a second
/// connection presenting the same token is refused.
#[test]
fn an_attach_token_opens_exactly_one_channel() {
    let account = TestAccount::new("token-single-use");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let started = start_session(&mut client, 1, &["/bin/sh", "-c", "sleep 30"]);
    let session = session_id(&started);
    let granted = attach_token(&mut client, 2, &session);
    let token = result(&granted)
        .get("attach_token")
        .and_then(Value::as_str)
        .expect("a token")
        .to_owned();

    let _first = open_channel(&account, &token);

    let mut second = RawClient::connect(&account.socket());
    let answer = second.say_hello_with_role(&token);
    assert_eq!(
        error_code(&answer),
        Some("protocol_violation"),
        "a spent token opened a second channel: {answer}"
    );
}

#[test]
fn a_forged_token_opens_nothing() {
    let account = TestAccount::new("token-forged");
    let _daemon = account.start_daemon();

    let mut client = RawClient::connect(&account.socket());
    let answer = client.say_hello_with_role(&"0".repeat(32));

    assert_eq!(error_code(&answer), Some("protocol_violation"), "{answer}");
}

/// Closing a channel is not ending a session. This is the product invariant
/// the whole daemon exists for: work outlives the surface watching it.
#[test]
fn detaching_leaves_the_session_running() {
    let account = TestAccount::new("detach-keeps-running");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let started = start_session(&mut client, 1, &["/bin/sh", "-c", "sleep 30"]);
    let session = session_id(&started);
    let granted = attach_token(&mut client, 2, &session);
    let token = result(&granted)
        .get("attach_token")
        .and_then(Value::as_str)
        .expect("a token")
        .to_owned();

    {
        let mut channel = open_channel(&account, &token);
        let _ = read_frame(&mut channel);
        // The channel drops here: the person walked away.
    }

    let listed = client
        .request(3, "session.list", None)
        .expect("session.list answered");
    let sessions = result(&listed)
        .get("sessions")
        .and_then(Value::as_array)
        .expect("a session array");

    assert_eq!(sessions.len(), 1, "detaching removed the session");
    assert_eq!(
        sessions[0].get("execution_state").and_then(Value::as_str),
        Some("running"),
        "detaching stopped the work"
    );
}

/// Input a client sends reaches the child, and what the child does with it
/// comes back on the screen.
#[test]
fn input_from_a_client_reaches_the_child() {
    let account = TestAccount::new("input-round-trip");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let started = start_session(
        &mut client,
        1,
        &[
            "/bin/sh",
            "-c",
            "read line; printf 'ECHOED-%s' \"$line\"; sleep 30",
        ],
    );
    let session = session_id(&started);
    let granted = attach_token(&mut client, 2, &session);
    let token = result(&granted)
        .get("attach_token")
        .and_then(Value::as_str)
        .expect("a token")
        .to_owned();

    let mut channel = open_channel(&account, &token);
    let _snapshot = read_frame(&mut channel);

    // An input frame: kind 3, epoch 0, sequence 0, then the bytes.
    let payload = b"typed-by-a-test\n";
    let mut frame = Vec::new();
    frame.push(3_u8);
    frame.extend_from_slice(&0_u64.to_be_bytes());
    frame.extend_from_slice(&0_u64.to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    channel
        .writer
        .write_all(&frame)
        .expect("the input was sent");

    // Ask for a fresh snapshot until the child's answer is on it.
    let resync = {
        let mut frame = Vec::new();
        frame.push(5_u8);
        frame.extend_from_slice(&0_u64.to_be_bytes());
        frame.extend_from_slice(&0_u64.to_be_bytes());
        frame.extend_from_slice(&0_u32.to_be_bytes());
        frame
    };

    let deadline = Instant::now() + SETTLE;
    let mut echoed = false;
    while Instant::now() < deadline && !echoed {
        channel
            .writer
            .write_all(&resync)
            .expect("the resync was sent");
        if let Some((_, payload)) = read_frame(&mut channel) {
            echoed = String::from_utf8_lossy(&payload).contains("ECHOED-typed-by-a-test");
        }
    }

    assert!(echoed, "what the client typed never reached the child");
}

/// The CLI's own path: `corral list` renders what the daemon serves.
#[test]
fn the_cli_lists_a_session_it_started() {
    let account = TestAccount::new("cli-list");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let _started = start_session(&mut client, 1, &["/bin/sh", "-c", "sleep 30"]);

    let listed = run(account.corral().arg("list"));

    assert!(listed.status.success(), "{}", stdout(&listed));
}
