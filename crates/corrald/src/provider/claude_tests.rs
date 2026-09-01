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
        assert_eq!(reported(payload).fact, expected);
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
    assert_eq!(report.fact, AgentFactKind::SessionStarted);
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
    assert_eq!(reported(&extended).fact, AgentFactKind::SessionStarted);
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
    assert_eq!(report.fact, AgentFactKind::TurnEnded);
    assert_eq!(report.identity, None);
}

/// The injected file declares its own hooks and nothing else. A settings file
/// that carried a model, a permission, or a `strict` flag would be Corral
/// wrapping provider-owned configuration, which ADR 0006 forbids.
///
/// `disableAllHooks` is not that: it does not configure the agent, it says the
/// file Corral is loading wants the hooks it just declared to run. Without it
/// a `disableAllHooks: true` in the person's own settings survives the merge
/// and silences them — measured on 2.1.251, and the whole injection becomes a
/// session Corral believes it is watching and is not.
#[test]
fn the_injected_settings_declare_hooks_and_nothing_else() {
    let document: serde_json::Value =
        serde_json::from_str(&settings_document("/opt/corral hook-relay --token abc"))
            .expect("the injected settings are JSON");
    let object = document.as_object().expect("an object");
    assert_eq!(object.len(), 2, "{document:#}");
    assert_eq!(
        object["disableAllHooks"],
        serde_json::json!(false),
        "{document:#}"
    );
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
        assert!(refusal.argument().starts_with("--settings"), "{refusal:?}");
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
        vec!["--safe-mode-ish".to_owned()],
    ] {
        assert_eq!(refuse_arguments(&allowed), Ok(()), "{allowed:?}");
    }
}

/// `--safe-mode` is refused for the same reason `--settings` is, and it is a
/// harder case: the injected file still loads and its hooks still never run.
///
/// Measured on 2.1.251 — the launch exits 0, the agent answers, and not one
/// hook fires. `disableAllHooks: false` does not rescue it, because this is
/// not a settings key. So the refusal is the only place it can be caught.
#[test]
fn safe_mode_is_refused_because_the_injection_cannot_survive_it() {
    for spelling in [
        vec!["--safe-mode".to_owned()],
        vec![
            "--model".to_owned(),
            "opus".to_owned(),
            "--safe-mode".to_owned(),
        ],
    ] {
        let refusal = refuse_arguments(&spelling).expect_err("refused");
        assert_eq!(refusal.argument(), "--safe-mode", "{refusal:?}");
        let said = refusal.to_string();
        assert!(said.contains("--safe-mode"), "{said}");
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
            "--settings",
            "/state/launch/corral-launch-y.json",
            "--resume",
            "d2dfcafd-9a73-4162-aa70-dddf99aa6e75",
        ],
    );
}

/// The word after `--resume` is a provider string, and `ExternalId` bounds its
/// length and refuses characters that hide or reorder text — nothing more.
/// Whatever a payload names, it lands after everything Corral needs, so no
/// value of it can displace the injection or take it as an argument.
#[test]
fn a_provider_id_that_reads_like_a_flag_cannot_reach_the_injection() {
    for hostile in ["--", "-p", "--settings", "--settings=/tmp/theirs"] {
        let argv = resume_argv(
            &ExternalId::new(hostile).expect("an external id this type accepts"),
            std::path::Path::new("/state/launch/corral-launch-y.json"),
        );
        let argv: Vec<String> = argv
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            &argv[..2],
            &["--settings", "/state/launch/corral-launch-y.json"],
            "{hostile:?} reached past the injection",
        );
        assert_eq!(argv.last().map(String::as_str), Some(hostile));
    }
}

/// The bypass this task closes: a *fresh* managed launch carrying the
/// provider's own attach argument reaches a conversation that already exists,
/// with neither the per-Session continuation claim nor the eligibility ladder
/// `session.resume` holds (ADR 0011 D1).
///
/// Every spelling this CLI accepts, measured on 2.1.251 — a refusal that knows
/// one spelling is a refusal a person walks around by accident.
#[test]
fn an_argument_that_joins_an_existing_conversation_is_refused() {
    for spelling in [
        vec!["--resume", "d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        vec!["--resume=d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        vec!["-r", "d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        // A short flag takes its value attached, and commander does not strip
        // the `=` — both still resume.
        vec!["-rd2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        vec!["-r=d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        // No value at all opens the picker, which is still an existing
        // conversation.
        vec!["--resume"],
        vec!["-r"],
        vec!["--continue"],
        vec!["-c"],
        // Short flags cluster, so `-c` hides inside one.
        vec!["-pc"],
        vec!["-cp"],
        vec!["--from-pr", "4210"],
        vec!["--from-pr=4210"],
        vec!["--cloud", "d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        // Options may precede it, and the refusal is about the argument
        // wherever it sits.
        vec!["--model", "opus", "--continue"],
    ] {
        let args: Vec<String> = spelling.iter().map(|word| (*word).to_owned()).collect();
        let refusal = refuse_arguments(&args).expect_err(&format!("refused: {spelling:?}"));
        assert!(
            matches!(
                refusal,
                ArgumentRefused::AttachesToAnExistingConversation(_)
            ),
            "{spelling:?} refused for the wrong reason: {refusal:?}",
        );
        assert!(
            refusal.to_string().contains(refusal.argument()),
            "{refusal:?}",
        );
    }
}

/// `attach <id>` opens a background session in this terminal, which is the
/// same harm wearing a subcommand. Commander dispatches a subcommand only as
/// the first argument — measured: `claude attach foo` answers "No job matching
/// 'foo'", while `claude -p attach foo` sends the words to the model — so
/// anywhere else the word is text.
#[test]
fn the_attach_subcommand_is_refused_where_this_cli_would_read_one() {
    let attaching = vec!["attach".to_owned(), "3f2a".to_owned()];
    assert!(matches!(
        refuse_arguments(&attaching).expect_err("refused"),
        ArgumentRefused::AttachesToAnExistingConversation(_)
    ));

    // Not first, so not a subcommand: this CLI would read it as prompt text.
    for theirs in [
        vec!["--model", "opus", "attach"],
        vec!["write the attach docs"],
        vec!["attach the file"],
    ] {
        let args: Vec<String> = theirs.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{theirs:?}");
    }
}

/// A value-taking letter eats the rest of its cluster, so a `c` or an `r`
/// after one is that flag's value and not a request to continue. Measured:
/// `-pc` continues, while `-nc`, `-dc`, and `-wc` do not.
#[test]
fn a_cluster_letter_after_a_value_taking_one_is_a_value() {
    for allowed in [
        vec!["-nc"],
        vec!["-dc"],
        vec!["-wc"],
        vec!["-nr"],
        vec!["-dr"],
        vec!["-wr"],
        // Alone it attaches nothing: the help says it works only with
        // `--resume` or `--continue`, and both of those are refused.
        vec!["--fork-session"],
    ] {
        let args: Vec<String> = allowed.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{allowed:?}");
    }
}

/// Everything after this CLI's own `--` is prompt text, including a word that
/// looks like a flag Corral refuses. Measured: `claude -- --resume <id>`
/// starts fresh and says the flag landed in the chat instead of the shell.
#[test]
fn nothing_after_the_separator_is_read_as_an_argument() {
    for theirs in [
        vec!["--", "--resume", "d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        vec!["--", "-c"],
        vec!["--", "--settings", "/tmp/theirs.json"],
        vec!["--", "attach"],
        vec!["--model", "opus", "--", "--continue"],
    ] {
        let args: Vec<String> = theirs.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{theirs:?}");
    }
}

/// A token consumed as a required option's value is not an argument at all,
/// and reading it as one fails in both directions.
///
/// Measured on 2.1.251: `claude -p --name -- --continue` answers
/// `--continue`'s own error, so `--name` took the separator as its value and
/// the flag after it parsed normally — a scan that stopped at that `--` would
/// wave through the attachment this refusal exists to stop. And
/// `claude -p -n -c` answers the plain input error, so `-n` took `-c` as a
/// name and there was nothing to refuse.
#[test]
fn a_word_a_required_option_swallows_is_not_read_as_an_argument() {
    for hidden in [
        // The separator itself, swallowed — so what follows is still options.
        vec!["--name", "--", "--continue"],
        vec![
            "--model",
            "--",
            "--resume",
            "d2dfcafd-9a73-4162-aa70-dddf99aa6e75",
        ],
        // The same swallowed separator hides the first ground too.
        vec!["--name", "--", "--settings", "/tmp/theirs.json"],
        vec!["-n", "--", "-c"],
        // A cluster whose value is attached leaves the next word a flag.
        vec!["-np", "-c"],
    ] {
        let args: Vec<String> = hidden.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{hidden:?}");
    }

    for theirs in [
        // The value happens to look like a flag Corral refuses, and is a name.
        vec!["-n", "-c"],
        vec!["--name", "--continue"],
        // A cluster ending in the required-value letter swallows the next word.
        vec!["-pn", "-c"],
        // An *optional* value takes no dash-leading word, so the separator
        // survives and everything after it is prompt text.
        vec!["-d", "--", "--continue"],
        vec![
            "--debug",
            "--",
            "--resume",
            "d2dfcafd-9a73-4162-aa70-dddf99aa6e75",
        ],
    ] {
        let args: Vec<String> = theirs.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{theirs:?}");
    }
}
