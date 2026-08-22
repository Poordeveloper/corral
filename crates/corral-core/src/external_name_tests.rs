use super::*;

#[test]
fn a_provider_name_is_kept_verbatim() {
    let provider = ProviderId::new("claude-code").expect("usable");

    assert_eq!(provider.as_str(), "claude-code");
    assert_eq!(provider.to_string(), "claude-code");
}

/// Provider data is untrusted input: a name that could rewrite a terminal
/// line or split a log record never reaches storage or display.
#[test]
fn a_control_character_is_refused() {
    let error = ExternalId::new("abc\u{1b}[2Kdef").expect_err("refused");

    assert_eq!(error.reason, NameRefusal::ControlCharacter);
}

#[test]
fn an_empty_name_is_refused() {
    let error = ProviderId::new("").expect_err("refused");

    assert_eq!(error.reason, NameRefusal::Empty);
}

#[test]
fn a_name_past_its_limit_is_refused() {
    let error = ToolName::new("t".repeat(ToolName::LIMIT + 1)).expect_err("refused");

    assert_eq!(
        error.reason,
        NameRefusal::TooLong {
            length: ToolName::LIMIT + 1,
            limit: ToolName::LIMIT,
        }
    );
    assert!(ToolName::new("t".repeat(ToolName::LIMIT)).is_ok());
}

/// The limit is in bytes, so a multi-byte name cannot slip past it by
/// counting characters.
#[test]
fn the_limit_counts_bytes() {
    let name = "é".repeat(ProviderId::LIMIT);

    assert!(ProviderId::new(name).is_err());
}
