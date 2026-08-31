use super::*;

fn arguments(words: &[&str]) -> Vec<OsString> {
    std::iter::once(OsString::from("/opt/corral"))
        .chain(words.iter().map(OsString::from))
        .collect()
}

#[test]
fn a_relay_invocation_is_recognised_without_a_parser() {
    let read = invocation(arguments(&[
        "hook-relay",
        "--provider",
        "claude",
        "--token",
        "0123456789abcdef0123456789abcdef",
    ]))
    .expect("a relay invocation");

    assert_eq!(read.provider, "claude");
    assert_eq!(read.token, "0123456789abcdef0123456789abcdef");
}

/// Skew is normal: an injected settings file names an absolute path and
/// invokes whatever is installed by the time an event fires (ADR 0004 D3). A
/// flag this build has no word for is an ordinary thing to meet, and meeting
/// one must not make the shim fail loudly — a parser answers an unrecognised
/// command line with usage on standard error and a non-zero exit, which Claude
/// Code reads as a blocking decision.
#[test]
fn arguments_from_a_later_build_are_ignored_rather_than_refused() {
    let read = invocation(arguments(&[
        "hook-relay",
        "--a-flag-from-later",
        "with-a-value",
        "--provider",
        "claude",
        "--token",
        "abc",
        "--another",
    ]))
    .expect("a relay invocation");

    assert_eq!(read.provider, "claude");
    assert_eq!(read.token, "abc");
}

/// A flag whose value is missing leaves the field empty rather than eating the
/// next flag. The daemon refuses what it cannot place, silently, as everything
/// on this path does.
#[test]
fn a_flag_without_its_value_leaves_the_field_empty() {
    let read = invocation(arguments(&["hook-relay", "--provider"])).expect("a relay invocation");

    assert_eq!(read.provider, "");
    assert_eq!(read.token, "");

    // Mid-argv, where the next word is another flag rather than the end. One
    // missing value must not consume the flag after it and take its value with
    // it.
    let read = invocation(arguments(&["hook-relay", "--provider", "--token", "abc"]))
        .expect("a relay invocation");

    assert_eq!(read.provider, "");
    assert_eq!(read.token, "abc");
}

#[test]
fn every_other_invocation_is_left_to_the_parser() {
    for words in [
        vec![],
        vec!["list"],
        vec!["new", "claude"],
        // The subcommand is the first word or it is not this invocation.
        vec!["new", "hook-relay"],
    ] {
        assert!(invocation(arguments(&words)).is_none(), "{words:?}");
    }
}

/// Absent flag, unchanged program: the relay reads standard input exactly as
/// it did before an argv-delivering provider existed.
#[test]
fn a_payload_comes_from_standard_input_unless_the_invocation_says_otherwise() {
    let read = invocation(arguments(&[
        "hook-relay",
        "--provider",
        "claude",
        "--token",
        "abc",
    ]))
    .expect("a relay invocation");

    assert!(matches!(read.payload, PayloadSource::Stdin));
}

/// Codex appends the notification JSON as one final argument and writes
/// nothing to standard input (ADR 0009 D2). The bytes are taken as they
/// arrived: the relay never parses a payload, so it never re-encodes one.
#[test]
fn an_argv_payload_is_the_final_argument_verbatim() {
    let payload = r#"{"type":"agent-turn-complete","thread-id":"01a0576f","last-assistant-message":"ok — \"done\""}"#;
    let read = invocation(arguments(&[
        "hook-relay",
        "--provider",
        "codex",
        "--token",
        "0123456789abcdef0123456789abcdef",
        "--payload-argv",
        payload,
    ]))
    .expect("a relay invocation");

    assert_eq!(read.provider, "codex");
    assert_eq!(read.token, "0123456789abcdef0123456789abcdef");
    match read.payload {
        PayloadSource::Argument(bytes) => assert_eq!(bytes, payload.as_bytes()),
        PayloadSource::Stdin => panic!("an argv payload was read as standard input"),
    }
}

/// The payload is the last word of the invocation, not the word after the
/// flag: a provider appends it after everything Corral wrote, and a later
/// build of either side may write more in between.
#[test]
fn an_argv_payload_is_taken_from_the_end_of_the_invocation() {
    let read = invocation(arguments(&[
        "hook-relay",
        "--payload-argv",
        "--provider",
        "codex",
        "--token",
        "abc",
        "{\"type\":\"agent-turn-complete\"}",
    ]))
    .expect("a relay invocation");

    assert_eq!(read.provider, "codex");
    assert_eq!(read.token, "abc");
    match read.payload {
        PayloadSource::Argument(bytes) => {
            assert_eq!(bytes, b"{\"type\":\"agent-turn-complete\"}");
        }
        PayloadSource::Stdin => panic!("an argv payload was read as standard input"),
    }
}

/// An invocation that declared an argv payload and carries none delivers
/// nothing. Standard input is not a fallback: a relay that read it here would
/// spend the whole interference budget waiting on a pipe the provider never
/// opened.
#[test]
fn a_declared_argv_payload_that_is_absent_is_not_sought_on_standard_input() {
    let read = invocation(arguments(&[
        "hook-relay",
        "--provider",
        "codex",
        "--token",
        "abc",
        "--payload-argv",
    ]))
    .expect("a relay invocation");

    match &read.payload {
        PayloadSource::Argument(bytes) => assert!(bytes.is_empty()),
        PayloadSource::Stdin => panic!("a declared argv payload fell back to standard input"),
    }
    // And it exits 0 without touching the socket, which is what every other
    // definite failure on this path does.
    assert_eq!(
        format!("{:?}", deliver(&read, Instant::now())),
        format!("{:?}", ExitCode::SUCCESS),
    );
}
