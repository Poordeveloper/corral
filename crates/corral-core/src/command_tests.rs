use super::*;

fn kind() -> CommandKind {
    CommandKind::new("session.create").expect("usable")
}

/// The order a producer describes a command in is an encoding detail, and
/// idempotency binds to meaning — so it cannot split one command into two.
#[test]
fn input_order_does_not_change_the_fingerprint() {
    let one = CommandFingerprint::builder(kind())
        .input("cwd", "/work")
        .input("provider", "claude-code")
        .build();
    let other = CommandFingerprint::builder(kind())
        .input("provider", "claude-code")
        .input("cwd", "/work")
        .build();

    assert_eq!(one, other);
}

#[test]
fn a_different_semantic_input_is_a_different_command() {
    let one = CommandFingerprint::builder(kind())
        .input("cwd", "/work")
        .build();
    let other = CommandFingerprint::builder(kind())
        .input("cwd", "/elsewhere")
        .build();

    assert_ne!(one, other);
}

#[test]
fn a_different_kind_is_a_different_command() {
    let create = CommandFingerprint::builder(kind()).build();
    let archive =
        CommandFingerprint::builder(CommandKind::new("session.archive").expect("usable")).build();

    assert_ne!(create, archive);
}

/// A value that looks like the canonical form's own punctuation must not be
/// able to impersonate a different set of inputs.
#[test]
fn an_input_value_cannot_forge_a_part_boundary() {
    let forged = CommandFingerprint::builder(kind())
        .input("a", "1i1:b1:2")
        .build();
    let genuine = CommandFingerprint::builder(kind())
        .input("a", "1")
        .input("b", "2")
        .build();

    assert_ne!(forged, genuine);
}

#[test]
fn naming_one_input_twice_describes_one_command() {
    let corrected = CommandFingerprint::builder(kind())
        .input("cwd", "/wrong")
        .input("cwd", "/work")
        .build();
    let direct = CommandFingerprint::builder(kind())
        .input("cwd", "/work")
        .build();

    assert_eq!(corrected, direct);
}

#[test]
fn a_fingerprint_round_trips_through_its_canonical_form() {
    let fingerprint = CommandFingerprint::builder(kind())
        .input("cwd", "/work")
        .build();

    assert_eq!(
        CommandFingerprint::from_canonical(fingerprint.as_str()),
        fingerprint
    );
}

#[test]
fn a_command_id_with_whitespace_is_refused() {
    assert_eq!(
        CommandId::new("two words").expect_err("refused"),
        MalformedCommandId::UnusableCharacter
    );
    assert_eq!(
        CommandId::new("").expect_err("refused"),
        MalformedCommandId::Empty
    );
    assert!(CommandId::new("019a4f1e-4a9b-7c2d-9f3e-2b6a1c0d5e77").is_ok());
}

#[test]
fn a_command_id_past_its_limit_is_refused() {
    let error = CommandId::new("c".repeat(CommandId::LIMIT + 1)).expect_err("refused");

    assert_eq!(
        error,
        MalformedCommandId::TooLong {
            length: CommandId::LIMIT + 1,
            limit: CommandId::LIMIT,
        }
    );
}
