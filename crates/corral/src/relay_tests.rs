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
