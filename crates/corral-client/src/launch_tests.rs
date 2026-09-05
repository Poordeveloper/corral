use super::*;

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn an_agent_name_is_a_provider_request() {
    assert_eq!(
        requested(&words("claude")),
        Some(Requested::Provider {
            name: "claude".to_owned(),
            args: Vec::new(),
        })
    );
}

/// The agent's own arguments pass through, with or without the separator a
/// person may type before them — and a separator among them cannot start a
/// second list.
#[test]
fn an_agents_own_arguments_pass_through() {
    let expected = Some(Requested::Provider {
        name: "codex".to_owned(),
        args: vec!["--model".to_owned(), "o3".to_owned()],
    });
    assert_eq!(requested(&words("codex --model o3")), expected);
    assert_eq!(requested(&words("codex -- --model o3")), expected);
    assert_eq!(
        requested(&words("codex -- --model -- o3")),
        Some(Requested::Provider {
            name: "codex".to_owned(),
            args: vec!["--model".to_owned(), "--".to_owned(), "o3".to_owned()],
        })
    );
}

/// The two namespaces stay apart all the way down: a program is what follows
/// the separator, and an unrecognised first word is still an agent request
/// the daemon refuses by name rather than a command this client guessed at.
#[test]
fn a_command_follows_the_separator_and_nothing_is_guessed() {
    assert_eq!(
        requested(&words("-- bash -l")),
        Some(Requested::Command(vec!["bash".to_owned(), "-l".to_owned()]))
    );
    assert_eq!(
        requested(&words("bash")),
        Some(Requested::Provider {
            name: "bash".to_owned(),
            args: Vec::new(),
        })
    );
}

#[test]
fn nothing_and_a_bare_separator_request_nothing() {
    assert_eq!(requested(&[]), None);
    assert_eq!(requested(&words("--")), None);
}
