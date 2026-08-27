use super::*;

/// The payloads Claude Code actually wrote, captured first-party. A parser
/// proven against invented JSON is proven against the test author.
const SESSION_START: &str = include_str!("../../fixtures/claude-hooks/SessionStart-startup.json");
const SESSION_START_RESUME: &str =
    include_str!("../../fixtures/claude-hooks/SessionStart-resume.json");
const SESSION_START_OLDER: &str =
    include_str!("../../fixtures/claude-hooks/SessionStart-2.1.239.json");
const USER_PROMPT_SUBMIT: &str = include_str!("../../fixtures/claude-hooks/UserPromptSubmit.json");
const STOP: &str = include_str!("../../fixtures/claude-hooks/Stop.json");
const NOTIFICATION: &str = include_str!("../../fixtures/claude-hooks/Notification.json");
const SESSION_END: &str = include_str!("../../fixtures/claude-hooks/SessionEnd.json");

fn reported(payload: &str) -> ProviderReport {
    interpret(payload).expect("a captured payload is interpretable")
}

#[test]
fn every_injected_event_normalizes_to_the_fact_it_names() {
    for (payload, expected) in [
        (SESSION_START, AgentFactKind::SessionStarted),
        (SESSION_START_RESUME, AgentFactKind::SessionStarted),
        (USER_PROMPT_SUBMIT, AgentFactKind::TurnStarted),
        (STOP, AgentFactKind::TurnEnded),
        (NOTIFICATION, AgentFactKind::AwaitingInput),
        (SESSION_END, AgentFactKind::SessionEnded),
    ] {
        assert_eq!(reported(payload).fact, Some(expected));
    }
}

#[test]
fn every_injected_event_carries_the_provider_session_id() {
    for payload in [
        SESSION_START,
        SESSION_START_RESUME,
        USER_PROMPT_SUBMIT,
        STOP,
        NOTIFICATION,
        SESSION_END,
    ] {
        assert!(
            reported(payload).identity.is_some(),
            "a captured payload names its session",
        );
    }
}

/// A resume keeps the identity: the same `session_id` arrives with
/// `source: resume`, which is what makes NativeResume recognizable by binding
/// uniqueness rather than by a continuity heuristic (S2; matrix scenario 2).
#[test]
fn a_resumed_start_reports_the_same_identity_as_the_first_one() {
    assert_eq!(
        reported(SESSION_START).identity,
        reported(SESSION_START_RESUME).identity
    );
}

/// The version matrix's whole point, in one test: a payload from a supported
/// earlier version carries fewer fields, and the parser needs none of them.
#[test]
fn an_older_versions_payload_is_read_by_this_build() {
    let report = reported(SESSION_START_OLDER);
    assert_eq!(report.fact, Some(AgentFactKind::SessionStarted));
    assert_eq!(
        report.identity.as_ref().map(ExternalId::as_str),
        Some("e670c1cf-1b2a-4c33-9d55-0f6a8c1b2d34"),
    );
}

/// Future input: a payload gaining fields must keep meaning what it meant.
#[test]
fn unknown_payload_fields_are_ignored() {
    let extended = SESSION_START.replace(
        "\"hook_event_name\"",
        "\"a_field_from_later\": {\"nested\": [1, 2]}, \"hook_event_name\"",
    );
    assert_eq!(
        reported(&extended).fact,
        Some(AgentFactKind::SessionStarted)
    );
}

/// Future input: a hook Corral does not inject, or one a later version adds.
/// Tolerated, and asserting nothing — not even the identity it carries.
#[test]
fn an_unknown_event_kind_asserts_nothing() {
    let unknown = SESSION_START.replace("\"SessionStart\"", "\"PreCompact\"");
    assert_eq!(interpret(&unknown), Err(Uninterpretable::UnknownEvent));
}

#[test]
fn a_payload_that_is_not_json_is_diagnostics() {
    for payload in ["", "not json", "[]", "{\"hook_event_name\": 7}"] {
        assert_eq!(interpret(payload), Err(Uninterpretable::Malformed));
    }
}

/// A known event whose id is missing or unusable is still the fact it names.
/// Losing a turn boundary over a field Corral does not need would throw away
/// evidence the launch token already places.
#[test]
fn a_known_event_without_a_usable_id_still_reports_its_fact() {
    let anonymous = STOP.replace(
        "\"session_id\": \"d2dfcafd-9a73-4162-aa70-dddf99aa6e75\"",
        "\"session_id\": \"\"",
    );
    let report = reported(&anonymous);
    assert_eq!(report.fact, Some(AgentFactKind::TurnEnded));
    assert_eq!(report.identity, None);
}

/// The injected file declares hooks and nothing else. A settings file that
/// carried a model, a permission, or a `strict` flag would be Corral wrapping
/// provider-owned configuration, which ADR 0006 forbids.
#[test]
fn the_injected_settings_declare_hooks_and_nothing_else() {
    let document: serde_json::Value =
        serde_json::from_str(&settings_document("/opt/corral hook-relay --token abc"))
            .expect("the injected settings are JSON");
    let object = document.as_object().expect("an object");
    assert_eq!(object.len(), 1, "{document:#}");
    let hooks = object["hooks"].as_object().expect("a hooks table");
    let mut declared: Vec<&String> = hooks.keys().collect();
    declared.sort();
    let mut injected: Vec<&str> = INJECTED.iter().map(|(name, _)| *name).collect();
    injected.sort_unstable();
    assert_eq!(declared, injected);
}

/// High-frequency hooks are deliberately not injected: nothing in this phase
/// consumes them, and adding one later is additive (ADR 0004 D6).
#[test]
fn tool_use_hooks_are_not_injected() {
    let document = settings_document("relay");
    assert!(!document.contains("PreToolUse"), "{document}");
    assert!(!document.contains("PostToolUse"), "{document}");
}

/// The injection goes first, ahead of anything the caller passed, and nothing
/// a caller writes after it can reach it — not a separator, not a flag looking
/// for a value (matrix scenarios 10 and 12).
#[test]
fn the_injected_settings_are_the_first_word_in_the_argv() {
    let argv = launch_argv(
        std::path::Path::new("/state/launch/corral-launch-x.json"),
        &["--".to_owned(), "--model".to_owned(), "opus".to_owned()],
    );
    let argv: Vec<String> = argv
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        argv,
        vec![
            "--settings",
            "/state/launch/corral-launch-x.json",
            "--",
            "--model",
            "opus",
        ],
    );
}

/// Position holds against any spelling; this is what turns the one spelling
/// Corral can recognise into an error a person can act on, rather than a
/// settings file of theirs quietly not loading.
#[test]
fn a_caller_supplied_settings_flag_is_refused_rather_than_dropped() {
    for spelling in [
        vec!["--settings".to_owned(), "/tmp/theirs.json".to_owned()],
        vec!["--settings=/tmp/theirs.json".to_owned()],
        vec![
            "--model".to_owned(),
            "opus".to_owned(),
            "--settings".to_owned(),
        ],
        // The flag also takes a JSON string rather than a path, and the
        // refusal is about the flag either way.
        vec!["--settings".to_owned(), "{\"hooks\":{}}".to_owned()],
    ] {
        let refusal = refuse_arguments(&spelling).expect_err("refused");
        assert!(refusal.argument.starts_with("--settings"), "{refusal:?}");
        // What a person reads names what they typed and why it is Corral's.
        let said = refusal.to_string();
        assert!(said.contains("--settings"), "{said}");
    }
}

/// Everything else a person may want to pass to their own agent is theirs.
///
/// `--setting-sources` earns its place here rather than being assumed: it
/// restricts which of the user's own settings files load, and it was driven
/// first-party alongside `--settings` to confirm the injected file still
/// applies (matrix scenario 11). An allow-list resting on an undriven
/// assumption would bless the same silent-unmanaged failure the two refusals
/// above exist to prevent.
#[test]
fn ordinary_provider_arguments_pass_through() {
    for allowed in [
        vec![],
        vec!["--model".to_owned(), "opus".to_owned()],
        vec!["--setting-sources".to_owned(), "user".to_owned()],
        vec!["--add-dir".to_owned(), "/work".to_owned()],
        // The separator is the caller's too: with Corral's pair already ahead
        // of it, everything after it is the caller's own prompt text and none
        // of Corral's business (matrix scenario 12).
        vec!["--".to_owned(), "a prompt".to_owned()],
        // A value that merely starts the same way is not the flag.
        vec!["--settings-are-fine".to_owned()],
    ] {
        assert_eq!(refuse_arguments(&allowed), Ok(()), "{allowed:?}");
    }
}

#[test]
fn a_resume_names_the_provider_session_and_the_injected_settings() {
    let argv = resume_argv(
        &ExternalId::new("d2dfcafd-9a73-4162-aa70-dddf99aa6e75").expect("usable"),
        std::path::Path::new("/state/launch/corral-launch-y.json"),
    );
    let argv: Vec<String> = argv
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        vec![
            "--resume",
            "d2dfcafd-9a73-4162-aa70-dddf99aa6e75",
            "--settings",
            "/state/launch/corral-launch-y.json",
        ],
    );
}
