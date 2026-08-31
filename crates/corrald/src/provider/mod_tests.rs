use super::*;

/// The provider namespace is closed, and an unknown name is refused rather
/// than treated as a program to run (grill Q6).
#[test]
fn an_unknown_product_is_not_a_provider() {
    assert_eq!(
        KnownProvider::from_name("claude"),
        Some(KnownProvider::Claude)
    );
    assert_eq!(
        KnownProvider::from_name("codex"),
        Some(KnownProvider::Codex)
    );
    for name in ["bash", "Claude", "claude-code", "", "Codex", "codex-cli"] {
        assert_eq!(KnownProvider::from_name(name), None, "{name}");
    }
}

/// The name a client sends and the name a binding stores are the same string,
/// through one owner. Two spellings would be two provider namespaces.
#[test]
fn a_provider_round_trips_through_its_wire_name() {
    for provider in KnownProvider::ALL {
        assert_eq!(KnownProvider::from_name(provider.as_str()), Some(provider));
    }
}

/// The reserved namespace records who minted an identity, and it is never the
/// provider a session runs (ADR 0008 D3).
#[test]
fn no_provider_claims_the_reserved_corral_namespace() {
    for provider in KnownProvider::ALL {
        assert_ne!(
            provider.as_str(),
            corral_core::ProviderId::RESERVED_FOR_CORRAL,
        );
    }
}

/// Every normalized fact has a provider-neutral wire spelling, and no two
/// share one: a client that could not tell two facts apart would render the
/// wrong sentence for one of them.
#[test]
fn every_fact_has_its_own_wire_spelling() {
    let kinds = [
        AgentFactKind::SessionStarted,
        AgentFactKind::TurnStarted,
        AgentFactKind::TurnEnded,
        AgentFactKind::AwaitingInput,
        AgentFactKind::SessionEnded,
    ];
    let mut spellings: Vec<String> = kinds
        .iter()
        .map(|kind| kind.as_wire().as_str().to_owned())
        .collect();
    spellings.sort();
    spellings.dedup();
    assert_eq!(spellings.len(), kinds.len());
}

/// No provider event name reaches a client. The wire vocabulary is Corral's,
/// and this is the assertion that keeps layer 3 provider-neutral
/// (ADR 0004 D3).
#[test]
fn no_wire_spelling_is_a_provider_event_name() {
    let provider_names = [
        "SessionStart",
        "UserPromptSubmit",
        "Stop",
        "Notification",
        "SessionEnd",
        "agent-turn-complete",
    ];
    for kind in [
        AgentFactKind::SessionStarted,
        AgentFactKind::TurnStarted,
        AgentFactKind::TurnEnded,
        AgentFactKind::AwaitingInput,
        AgentFactKind::SessionEnded,
    ] {
        let spelling = kind.as_wire().as_str().to_owned();
        assert!(
            !provider_names.contains(&spelling.as_str()),
            "{spelling} is a provider's own event name",
        );
    }
}

/// Interpretation dispatches on the provider a launch was created as, and a
/// payload that is not that provider's shape is diagnostics rather than a
/// guess at another provider's format.
#[test]
fn interpretation_dispatches_on_the_launch_provider() {
    assert_eq!(
        interpret(KnownProvider::Claude, "{\"type\":\"agent-turn-complete\"}"),
        Err(Uninterpretable::Malformed),
    );
    assert_eq!(
        interpret(KnownProvider::Codex, "{\"hook_event_name\":\"Stop\"}"),
        Err(Uninterpretable::Malformed),
    );
}

/// How a provider hands over a payload is a measured fact about that provider,
/// and it is what the relay is told (ADR 0009 D2). A provider whose delivery
/// this got wrong would fire hooks a relay waits for on the wrong channel.
#[test]
fn each_provider_declares_where_its_payload_arrives() {
    assert_eq!(
        KnownProvider::Claude.payload_delivery(),
        PayloadDelivery::Stdin
    );
    assert_eq!(
        KnownProvider::Codex.payload_delivery(),
        PayloadDelivery::FinalArgument
    );
}
