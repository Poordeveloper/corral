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
        // A word that merely starts the same way is not the flag, and is
        // refused for its own reason rather than that one.
        vec!["--".to_owned(), "--settings-are-fine".to_owned()],
    ] {
        assert_eq!(refuse_arguments(&allowed), Ok(()), "{allowed:?}");
    }

    // Before the terminator it is an option this build cannot read, which is
    // now its own refusal: a prefix of a refused flag is not that flag, and
    // an unvalidated option is refused whatever it resembles (ADR 0012 D1).
    for unread in [
        vec!["--settings-are-fine".to_owned()],
        vec!["--safe-mode-ish".to_owned()],
    ] {
        let refusal = refuse_arguments(&unread).expect_err("refused");
        assert!(
            matches!(refusal, ArgumentRefused::NotValidatedForAManagedLaunch(_)),
            "{unread:?} refused for the wrong reason: {refusal:?}",
        );
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

/// `--help` is not this CLI's inventory, and a refusal built from it is a
/// refusal with holes.
///
/// `--remote` appears nowhere in the help and is a deprecated alias for
/// `--cloud` — measured, it answers with `--cloud`'s own name. `--teleport` is
/// in the help, but only its one-line description says what it does, and the
/// binary's own text puts it in the family: "`--environment` cannot be
/// combined with `--resume`, `--continue`, or `--teleport`", and "/teleport
/// pulls a cloud session into a terminal on your own machine".
#[test]
fn an_attaching_flag_the_help_does_not_explain_is_still_one() {
    for spelling in [
        vec!["--remote", "d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        vec!["--remote=d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
        vec!["--teleport"],
        vec!["--teleport", "d2dfcafd-9a73-4162-aa70-dddf99aa6e75"],
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
    }
}

/// The separator-swallow bypass, reached through a required-value flag the
/// help does not list.
///
/// Measured: `claude -p --append-subagent-system-prompt -- --continue` answers
/// `--continue`'s own error, so the option took the separator and the flag
/// behind it parsed. Every root option that requires a value is in the table
/// for this reason — one missing entry is one bypass.
#[test]
fn a_required_value_flag_outside_the_help_still_swallows_the_separator() {
    for hidden in [
        vec!["--append-subagent-system-prompt", "--", "--continue"],
        vec!["--system-prompt-file", "--", "--resume"],
        vec![
            "--permission-prompt-tool",
            "--",
            "--settings",
            "/tmp/theirs.json",
        ],
        vec!["--ref", "--", "--continue"],
    ] {
        let args: Vec<String> = hidden.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{hidden:?}");
    }
}

/// The regression the founder ruling asks for by name: an option this build
/// cannot read, placed before any attaching one, ends in a refused launch and
/// never in an attached session (ADR 0012 D1).
///
/// It is the table-ageing case made concrete. Whichever way a later release
/// spells an attachment, and whatever new option precedes it, the launch stops
/// here — the safety no longer rests on the attaching list having found
/// everything.
#[test]
fn an_unread_option_before_an_attaching_one_refuses_the_launch() {
    for attaching in [
        "--continue",
        "--resume",
        "--teleport",
        "--cloud",
        "--remote",
        "--from-pr",
    ] {
        for shape in [
            vec!["--an-option-from-a-later-release", attaching],
            // The shape that made this a decision: the unknown option may be
            // the one that swallows the terminator.
            vec!["--an-option-from-a-later-release", "--", attaching],
            vec!["-Z", "--", attaching],
        ] {
            let args: Vec<String> = shape.iter().map(|word| (*word).to_owned()).collect();
            assert!(
                refuse_arguments(&args).is_err(),
                "{shape:?} must not reach a managed launch",
            );
        }
    }
}

/// `--` is an option terminator only where the grammar establishes the parser
/// is in a state to read it as one (ADR 0012 D3). Permanent: it is the
/// assumption that produced two separate bypasses.
///
/// Measured, `--append-subagent-system-prompt -- --continue` hands the
/// terminator to the option and continues a conversation; the same three words
/// after a valueless flag are a prompt.
#[test]
fn the_terminator_is_only_a_terminator_where_the_grammar_says_so() {
    // Awaiting a required value: the terminator is that value, and the words
    // behind it are options again.
    for swallowed in [
        vec!["--append-subagent-system-prompt", "--", "--continue"],
        vec!["--name", "--", "--resume"],
        vec!["-n", "--", "-c"],
    ] {
        let args: Vec<String> = swallowed.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{swallowed:?}");
    }

    // Not awaiting anything: a terminator, and everything behind it is data.
    for terminated in [
        vec!["--print", "--", "--continue"],
        vec!["--name", "session", "--", "--continue"],
        vec!["--model=opus", "--", "--resume"],
        vec!["--", "--continue"],
    ] {
        let args: Vec<String> = terminated.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{terminated:?}");
    }
}

/// The property that makes an ageing table a source of friction rather than of
/// holes: a flag this build cannot read is not assumed to take nothing.
///
/// After a known valueless flag, an inline-value flag, or an ordinary word, a
/// `--` is the separator and everything behind it is prompt text. After a flag
/// this build has never heard of, it is not: commander would hand it over if
/// that flag wanted a value, and the words behind it would be options again.
/// So the scan reads on, and the cost is refusing a prompt written after an
/// unknown flag rather than missing an attachment behind one.
#[test]
fn an_unreadable_flag_does_not_make_the_next_word_safe() {
    for conservative in [
        vec!["--a-flag-from-a-later-release", "--", "--continue"],
        vec![
            "--a-flag-from-a-later-release",
            "--",
            "--settings",
            "/tmp/x.json",
        ],
        // An unknown letter makes the whole cluster unreadable.
        vec!["-z", "--", "--continue"],
    ] {
        let args: Vec<String> = conservative.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{conservative:?}");
    }

    for known in [
        // Measured valueless, so the separator is one.
        vec!["--print", "--", "--continue"],
        vec!["-p", "--", "--continue"],
        vec!["--verbose", "--", "--resume"],
        // Its value is inside the word, so it reaches no further.
        vec!["--model=opus", "--", "--continue"],
        vec!["-nname", "--", "--continue"],
        // An ordinary word reaches nothing at all.
        vec!["hello", "--", "--continue"],
    ] {
        let args: Vec<String> = known.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{known:?}");
    }
}

/// The token-role sweep, mechanically over every form this grammar has to
/// place a word in.
///
/// The invariant under test is the one two bypasses came from ignoring:
///
/// > a token becomes data only because the verified grammar says parsing has
/// > transitioned to data, never merely because of its spelling or position.
///
/// So each row is a shape rather than an example, and the expectation is about
/// the role the parser gives the word — not about what it looks like.
#[test]
fn every_token_form_is_placed_by_the_grammar_rather_than_by_its_spelling() {
    // A word whose role is "value" or "data" cannot refuse, however it is
    // spelled.
    for data in [
        // A required value, joined and separated.
        vec!["--name=--continue"],
        vec!["--name", "--continue"],
        // A short flag's value is the rest of its own word, so what follows is
        // an ordinary word again.
        vec!["-nname", "hello"],
        vec!["-p", "hello"],
        // Repeated options, each read on its own.
        vec!["--name", "one", "--name", "two"],
        vec!["--model", "opus", "--model", "sonnet"],
        // A required value left missing is the provider's error to raise, not
        // Corral's to guess at: nothing here is refused for it.
        vec!["--name"],
        vec!["--model", "--name"],
        // `--` sitting in a required-value slot is that value.
        vec!["--name", "--"],
        // Past a terminator the grammar established, every word is data —
        // including ones that would be refused anywhere before it.
        vec![
            "--",
            "--continue",
            "--resume",
            "--settings",
            "--zzz-unknown",
        ],
        vec!["--print", "--", "--teleport"],
    ] {
        let args: Vec<String> = data.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{data:?}");
    }

    // A word whose role is "option" is judged, in every spelling of it.
    for option in [
        (vec!["--continue"], "attaching, bare"),
        (vec!["--resume=x"], "attaching, joined value"),
        (vec!["-rx"], "attaching, short with attached value"),
        (vec!["-pc"], "attaching, inside a cluster"),
        (vec!["--settings=x"], "competing, joined value"),
        (vec!["--zzz-unknown"], "unread, long"),
        (vec!["--zzz-unknown=1"], "unread, long with a joined value"),
        (vec!["-Z"], "unread, short"),
        (
            vec!["--name", "one", "--continue"],
            "after a consumed value",
        ),
        (
            vec!["-nname", "--continue"],
            "after a short flag that carried its value inside its own word",
        ),
        (vec!["--print", "--continue"], "after a valueless option"),
        (
            vec!["hello", "--continue"],
            "after a positional, which does not end parsing",
        ),
    ] {
        let (shape, why) = option;
        let args: Vec<String> = shape.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{shape:?} ({why})");
    }
}
