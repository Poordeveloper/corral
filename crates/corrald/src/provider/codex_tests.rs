use super::*;

/// The payloads Codex actually wrote, captured first-party. A parser proven
/// against invented JSON is proven against the test author.
const TURN_COMPLETE_TUI: &str =
    include_str!("../../fixtures/codex-notify/agent-turn-complete-tui.json");
const TURN_COMPLETE_EXEC: &str =
    include_str!("../../fixtures/codex-notify/agent-turn-complete-exec.json");

fn relay() -> RelayInvocation {
    RelayInvocation::of_words(
        "/opt/corral/corral",
        &[
            "hook-relay",
            "--provider",
            "codex",
            "--token",
            "0123456789abcdef0123456789abcdef",
            "--payload-argv",
        ],
    )
}

fn words(argv: &[OsString]) -> Vec<String> {
    argv.iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect()
}

fn fresh(args: &[&str]) -> Vec<String> {
    let intent = LaunchIntent::Fresh {
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
    };
    words(&compose_launch(&intent, &relay()).argv)
}

fn reported(payload: &str) -> ProviderReport {
    interpret(payload).expect("a captured payload is interpretable")
}

/// The one notification family Codex fires today, from both clients that fire
/// it. `client` is read by nothing: which surface produced a turn is not a
/// fact this adapter is entitled to decide anything on.
#[test]
fn a_completed_turn_normalizes_to_the_fact_it_names() {
    for payload in [TURN_COMPLETE_TUI, TURN_COMPLETE_EXEC] {
        assert_eq!(reported(payload).fact, AgentFactKind::TurnEnded);
        assert!(reported(payload).identity.is_some(), "{payload}");
    }
}

/// No start is reported, so no origin exists — and `None` rather than
/// `Unrecognized`, because unreported and unrecognizable are different facts
/// (ADR 0009 D3).
#[test]
fn nothing_codex_reports_carries_an_origin() {
    for payload in [TURN_COMPLETE_TUI, TURN_COMPLETE_EXEC] {
        assert_eq!(reported(payload).origin, None, "{payload}");
    }
}

/// A type this build has no word for is tolerated and asserts nothing — not
/// even the identity it happens to carry (ADR 0004 D3).
#[test]
fn a_notification_from_a_later_release_asserts_nothing() {
    let later =
        r#"{"type":"agent-turn-started","thread-id":"01a0576f-0ecc-7b21-9719-f38f9e4ef933"}"#;

    assert_eq!(interpret(later), Err(Uninterpretable::UnknownEvent));
}

#[test]
fn a_payload_that_is_not_this_shape_is_malformed_rather_than_a_fact() {
    for payload in [
        "",
        "not json at all",
        "[]",
        r#"{"thread-id":"01a0576f-0ecc-7b21-9719-f38f9e4ef933"}"#,
        r#"{"type":42}"#,
    ] {
        assert_eq!(
            interpret(payload),
            Err(Uninterpretable::Malformed),
            "{payload}"
        );
    }
}

/// A known notification with no usable id is a fact without an identity. The
/// fact is still true of the launch its token names, and refusing the whole
/// event would lose it over a field Corral does not need to know what
/// happened.
#[test]
fn a_completed_turn_without_an_identity_is_still_a_completed_turn() {
    let anonymous = r#"{"type":"agent-turn-complete","cwd":"/work/demo"}"#;
    let report = interpret(anonymous).expect("a fact without an identity");

    assert_eq!(report.fact, AgentFactKind::TurnEnded);
    assert_eq!(report.identity, None);
}

/// An id Corral will not hold is refused rather than held, and the fact
/// survives it.
#[test]
fn an_identity_corral_cannot_hold_is_refused_and_the_fact_kept() {
    // A right-to-left override in the middle of an id: `ExternalId` refuses
    // characters that hide or reorder the text they are rendered into.
    let hostile = "{\"type\":\"agent-turn-complete\",\"thread-id\":\"01a0\u{202e}576f\"}";
    let report = interpret(hostile).expect("a fact");

    assert_eq!(report.fact, AgentFactKind::TurnEnded);
    assert_eq!(report.identity, None);
}

/// The override goes first, before anything the caller passed: nothing Corral
/// needs may sit where caller input can reach it (spike scenario 5).
#[test]
fn the_notify_override_precedes_every_caller_argument() {
    let argv = fresh(&["--", "--model", "gpt-5"]);

    assert_eq!(argv[0], "-c");
    assert!(argv[1].starts_with("notify=["), "{argv:?}");
    assert_eq!(&argv[2..], ["--", "--model", "gpt-5"]);
}

/// The whole invocation, program first, in the order the relay recognises
/// itself by — including the flag that says the payload rides argv.
#[test]
fn the_override_names_the_whole_relay_invocation() {
    let argv = fresh(&[]);

    assert_eq!(
        argv[1],
        r#"notify=["/opt/corral/corral","hook-relay","--provider","codex","--token","0123456789abcdef0123456789abcdef","--payload-argv"]"#,
    );
}

/// Corral owns the quoting because the relay's path is a filesystem path, and
/// a value that does not round-trip is a launch that looks managed and can
/// never report.
///
/// Decoded with a JSON parser, deliberately. The escape vocabulary emitted
/// here — `\"`, `\\`, the five named control escapes, and `\uXXXX` — is the
/// part TOML basic strings and JSON strings spell identically, so a JSON
/// decode is a real parser reading it rather than this test unescaping it
/// itself. An escape outside that intersection would fail here, which is the
/// signal wanted: it would mean the value had left the shared ground. That
/// real Codex accepts it is the matrix's to prove, not a unit test's.
#[test]
fn a_relay_path_survives_being_a_toml_string() {
    for awkward in [
        "/opt/my corral/corral",
        r#"/opt/"quoted"/corral"#,
        r"/opt/back\slash/corral",
        "/opt/nul\u{1}control/corral",
        "/opt/新建文件夹/corral",
    ] {
        let quoted = toml_string(awkward);
        let decoded: String =
            serde_json::from_str(&quoted).unwrap_or_else(|_| panic!("{quoted} decodes"));
        assert_eq!(decoded, awkward);
    }
}

/// A continuation is the same override with a fresh token, plus the verb the
/// provider itself prints on exit — and the override still goes first,
/// because the word after `resume` is a provider string.
#[test]
fn a_continuation_composes_the_resume_verb_behind_the_override() {
    let external_id = ExternalId::new("01a0576f-0ecc-7b21-9719-f38f9e4ef933").expect("an id");
    let intent = LaunchIntent::Continue { external_id };
    let argv = words(&compose_launch(&intent, &relay()).argv);

    assert_eq!(argv[0], "-c");
    assert!(argv[1].starts_with("notify=["), "{argv:?}");
    assert_eq!(
        &argv[2..],
        ["resume", "01a0576f-0ecc-7b21-9719-f38f9e4ef933"]
    );
}

/// The injection rides the argv, so there is no artifact and no file lifecycle
/// to own (ADR 0009 D1).
#[test]
fn a_codex_launch_leaves_nothing_on_disk() {
    for intent in [
        LaunchIntent::Fresh { args: Vec::new() },
        LaunchIntent::Continue {
            external_id: ExternalId::new("01a0576f-0ecc-7b21-9719-f38f9e4ef933").expect("an id"),
        },
    ] {
        assert!(compose_launch(&intent, &relay()).artifact.is_none());
    }
}

/// Every spelling this CLI accepts for a notify override, because the last one
/// on the invocation is the one that takes effect (spike scenario 5).
#[test]
fn a_caller_may_not_override_notify_in_any_spelling() {
    for spelling in [
        vec!["-c", "notify=[]"],
        vec!["--config", r#"notify=["theirs"]"#],
        vec!["-cnotify=[]"],
        vec!["-c=notify=[]"],
        vec!["--config=notify=[]"],
        vec!["-c", "notify.program=\"theirs\""],
        vec!["-m", "gpt-5", "-c", "notify=[]"],
    ] {
        let args: Vec<String> = spelling.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{spelling:?}");
    }
}

/// A managed Codex session is the interactive TUI and nothing else
/// (ADR 0009 D1, grill Q7). A subcommand starts a different program, and two
/// of them — `resume` and `fork` — attach a second process to a conversation
/// Corral may already be running, which is exactly what `session.resume`'s
/// continuation claim exists to prevent.
#[test]
fn a_caller_may_not_start_a_surface_corral_does_not_manage() {
    for outside in [
        vec!["resume", "01a0576f-0ecc-7b21-9719-f38f9e4ef933"],
        vec!["resume", "--last"],
        vec!["fork", "--last"],
        vec!["exec", "do a thing"],
        vec!["e", "do a thing"],
        vec!["app-server"],
        vec!["mcp-server"],
        vec!["login"],
        vec!["delete", "01a0576f-0ecc-7b21-9719-f38f9e4ef933"],
        // Options may precede the subcommand, so the first word is not the
        // only place one can hide.
        vec![
            "-m",
            "gpt-5",
            "resume",
            "01a0576f-0ecc-7b21-9719-f38f9e4ef933",
        ],
        vec!["--search", "exec", "do a thing"],
        // A boolean flag takes no value, so the word after it is still the
        // subcommand. Listing one as value-taking would wave this through.
        vec!["--oss", "resume", "01a0576f-0ecc-7b21-9719-f38f9e4ef933"],
        vec!["--no-alt-screen", "fork"],
    ] {
        let args: Vec<String> = outside.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_err(), "{outside:?}");
    }
}

/// Codex's own `--` ends its option parsing: what follows is the prompt
/// positional, and a word that looks like a flag or a subcommand there is
/// text. Measured, not assumed — `codex -- exec hi` answers
/// `unexpected argument 'hi'` against `codex [OPTIONS] [PROMPT]`, which is the
/// root command refusing a second positional rather than `exec` running
/// (matrix scenario 12).
///
/// Nothing after it can reach Corral's own override, which sits ahead of every
/// caller word, so there is nothing left to refuse.
#[test]
fn the_separator_hands_the_rest_to_the_agent_as_text() {
    for theirs in [
        vec!["--", "--config=notify=[]"],
        vec!["--", "-c", "notify=[]"],
        vec!["--", "resume", "01a0576f-0ecc-7b21-9719-f38f9e4ef933"],
        vec!["--", "exec"],
        vec!["-m", "gpt-5", "--", "fork"],
    ] {
        let args: Vec<String> = theirs.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{theirs:?}");
    }
}

/// A word that names a subcommand is only a subcommand where Codex would read
/// one. The value of a value-taking flag is a value — a directory called
/// `app`, a profile called `review` — and refusing those would be Corral
/// deciding how somebody's agent runs over a name collision.
#[test]
fn a_value_that_happens_to_name_a_subcommand_is_still_a_value() {
    for allowed in [
        vec!["-C", "app"],
        vec!["--cd", "debug"],
        vec!["--add-dir", "sandbox"],
        vec!["--profile", "review"],
        vec!["-m", "e"],
        vec!["-c", "model=\"gpt-5\"", "--profile", "apply"],
    ] {
        let args: Vec<String> = allowed.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{allowed:?}");
    }
}

/// Everything else a person may want to pass to their own agent is theirs,
/// including other configuration overrides, a profile, and the separator.
#[test]
fn everything_that_does_not_defeat_the_override_is_the_callers() {
    for allowed in [
        vec![],
        vec!["-m", "gpt-5"],
        vec!["-c", "model=\"gpt-5\""],
        vec!["--profile", "work"],
        vec!["-c", "notifications=true"],
        vec!["--", "--not-a-flag-of-ours"],
        vec!["-c"],
        vec!["--config"],
    ] {
        let args: Vec<String> = allowed.iter().map(|word| (*word).to_owned()).collect();
        assert!(refuse_arguments(&args).is_ok(), "{allowed:?}");
    }
}

/// The refusal names what the person wrote, so they can find it in their own
/// command line.
#[test]
fn a_refusal_names_the_argument_it_refused() {
    let args = vec!["-c".to_owned(), "notify=[]".to_owned()];
    let refused = refuse_arguments(&args).expect_err("a refusal");

    assert_eq!(refused.argument(), "notify=[]");
    assert!(refused.to_string().contains("notify=[]"));
}
