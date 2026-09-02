use super::*;
use corral_protocol::method::{
    AttentionFacts, AttentionItemFacts, AttentionReasonWire, AttentionWireState, LastKnownFacts,
};

fn listed(execution_state: &str, terminal_access: Option<TerminalAccess>) -> SessionListItem {
    SessionListItem {
        session_id: "0f9b6c1a-0000-0000-0000-000000000000".to_owned(),
        title: "sh".to_owned(),
        execution_state: execution_state.to_owned(),
        terminal_access,
        provider: None,
        agent_event: None,
        origin: None,
        location_hint: None,
        attention: None,
    }
}

/// A session the daemon reported provider facts about.
fn reported(kind: AgentEventKind, reported_at: SystemTime) -> SessionListItem {
    SessionListItem {
        provider: Some(corral_protocol::method::ProviderFacts {
            name: "claude".to_owned(),
            external_id: Some("d2dfcafd-9a73-4162-aa70-dddf99aa6e75".to_owned()),
        }),
        agent_event: Some(corral_protocol::method::AgentEvent {
            kind,
            at_ms: millis(reported_at),
        }),
        ..listed("running", Some(TerminalAccess::Available))
    }
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn millis(at: SystemTime) -> i64 {
    i64::try_from(
        at.duration_since(SystemTime::UNIX_EPOCH)
            .expect("after the epoch")
            .as_millis(),
    )
    .expect("representable")
}

/// The projection, in full. Table-driven because the regression this exists
/// to prevent is a single arm quietly promoting a runtime fact into a
/// semantic claim.
#[test]
fn execution_state_projects_onto_the_states_corral_may_claim() {
    let cases = [
        ("running", MainState::Unknown, "Running · Status unknown"),
        ("exited", MainState::Exited, "Exited"),
        (
            "unknown",
            MainState::Unknown,
            "Runtime unverified · Status unknown",
        ),
    ];

    for (execution_state, state, line) in cases {
        let presented = present(&listed(execution_state, None));

        assert_eq!(presented.state, state, "{execution_state}");
        assert_eq!(presented.state_line(), line, "{execution_state}");
    }
}

/// The invariant the module exists for. No execution state — including ones
/// this build has never seen — may produce a state that needs semantic
/// evidence nothing has yet.
#[test]
fn no_execution_state_manufactures_a_semantic_status() {
    for execution_state in [
        "running",
        "exited",
        "unknown",
        "working",
        "needs_you",
        "ready",
        "",
    ] {
        let line = present(&listed(execution_state, None)).state_line();

        for forbidden in ["Working", "Needs You", "Ready"] {
            assert!(
                !line.contains(forbidden),
                "{execution_state} produced {line:?}"
            );
        }
    }
}

/// A value from a newer daemon is unknown, not a fourth behaviour: the wire
/// contract says an unrecognised execution state is read as unknown, and the
/// surface must not quietly disagree.
#[test]
fn an_unrecognised_execution_state_is_shown_as_unknown() {
    let later = present(&listed("suspended", None));

    assert_eq!(later, present(&listed("unknown", None)));
}

/// Exited is the one main state execution truth may establish on its own, and
/// it is stated alone. "Exited · Status unknown" would say the session might
/// still need something, which it cannot.
#[test]
fn an_exited_session_says_nothing_about_status() {
    let line = present(&listed("exited", None)).state_line();

    assert_eq!(line, "Exited");
    assert!(!line.contains("unknown"), "{line}");
}

/// A screen Corral cannot serve is a secondary line and a refusal, and it
/// leaves the main state alone: the process may be running perfectly.
#[test]
fn an_unserveable_screen_is_secondary_and_refuses_open() {
    let presented = present(&listed("running", Some(TerminalAccess::Unavailable)));

    assert_eq!(presented.screen, Some("Screen unavailable"));
    assert_eq!(presented.refuses_open(), Some("Screen unavailable"));
    assert_eq!(
        presented.state_line(),
        "Running · Status unknown",
        "a capability fact leaked into the main state"
    );
}

/// The internal word never reaches a person, whatever the field says.
#[test]
fn nothing_a_surface_renders_says_poisoned() {
    for access in [
        Some(TerminalAccess::Available),
        Some(TerminalAccess::Unavailable),
        None,
    ] {
        let presented = present(&listed("running", access));
        let rendered = format!(
            "{} {}",
            presented.state_line(),
            presented.screen.unwrap_or("")
        );

        for forbidden in ["oison", "Broken", "Error"] {
            assert!(!rendered.contains(forbidden), "{rendered:?}");
        }
    }
}

/// Absence is not a refusal. A daemon that never sent the field, and one that
/// sent a word this build does not know, both leave Open on offer — the
/// answer comes from trying, not from a guess (`AGENTS.md` §Protocol).
#[test]
fn an_unknown_terminal_access_does_not_refuse_open() {
    for access in [Some(TerminalAccess::Available), None] {
        let presented = present(&listed("running", access));

        assert_eq!(presented.screen, None);
        assert_eq!(presented.refuses_open(), None);
    }
}

/// The example the design states, rendered: past tense, with provenance and
/// age (ADR 0004 D7).
#[test]
fn a_reported_fact_reads_as_a_report_with_its_age() {
    let presented = present_at(
        &reported(AgentEventKind::AwaitingInput, at(1_000)),
        at(1_300),
    );

    assert_eq!(
        presented.agent.as_deref(),
        Some("Claude reported waiting for input · 5m ago"),
    );
}

/// A second provider reaches this surface as data, not as code. The projection
/// names whichever agent the daemon said reported, and the sentence a Codex
/// session gets is the one the vocabulary already had — a turn ended, past
/// tense, with its age.
#[test]
fn a_second_providers_fact_renders_through_the_same_projection() {
    let item = SessionListItem {
        provider: Some(corral_protocol::method::ProviderFacts {
            name: "codex".to_owned(),
            external_id: Some("01a0576f-0ecc-7b21-9719-f38f9e4ef933".to_owned()),
        }),
        ..reported(AgentEventKind::TurnEnded, at(1_000))
    };

    let presented = present_at(&item, at(1_120));

    assert_eq!(
        presented.agent.as_deref(),
        Some("Codex reported finishing a turn · 2m ago"),
    );
    // The one fact Codex reports says nothing about a main state, exactly as
    // Claude's does not.
    assert_eq!(presented.state, MainState::Unknown);
}

/// The regression the whole phase turns on. No provider report — including the
/// one that most looks like it — produces Working, Needs You, or Ready, and
/// none of them touches the main state or the runtime fact beside it.
#[test]
fn no_reported_fact_manufactures_a_semantic_status() {
    for kind in [
        AgentEventKind::SessionStarted,
        AgentEventKind::TurnStarted,
        AgentEventKind::TurnEnded,
        AgentEventKind::AwaitingInput,
        AgentEventKind::SessionEnded,
        AgentEventKind::Unknown("needs_you".to_owned()),
    ] {
        let item = reported(kind.clone(), at(1_000));
        let presented = present_at(&item, at(1_010));

        assert_eq!(presented.state, MainState::Unknown, "{kind:?}");
        assert_eq!(
            presented.state_line(),
            "Running · Status unknown",
            "{kind:?}"
        );
        let line = presented.agent.unwrap_or_default();
        for forbidden in ["Working", "Needs You", "Ready", "needs_you"] {
            assert!(!line.contains(forbidden), "{kind:?} rendered {line:?}");
        }
    }
}

/// Every rendered sentence is in the past tense and names who said it. A line
/// that read as a claim about now would be the attention engine's job done
/// badly two phases early.
#[test]
fn every_reported_fact_names_its_source_and_stays_in_the_past() {
    for kind in [
        AgentEventKind::SessionStarted,
        AgentEventKind::TurnStarted,
        AgentEventKind::TurnEnded,
        AgentEventKind::AwaitingInput,
        AgentEventKind::SessionEnded,
    ] {
        let line = present_at(&reported(kind.clone(), at(1_000)), at(1_005))
            .agent
            .unwrap_or_else(|| panic!("{kind:?} renders a line"));

        assert!(line.starts_with("Claude reported "), "{line}");
        assert!(line.ends_with(" ago"), "{line}");
    }
}

/// A kind this build has no word for renders nothing at all: the client states
/// no claim it cannot name, and the raw provider spelling never reaches a
/// person (`AGENTS.md` §Protocol).
#[test]
fn a_fact_this_build_cannot_name_renders_nothing() {
    let item = reported(AgentEventKind::Unknown("compacted".to_owned()), at(1_000));

    let presented = present_at(&item, at(1_010));

    assert_eq!(presented.agent, None);
    assert!(
        !presented
            .beneath()
            .iter()
            .any(|line| line.contains("compacted"))
    );
}

/// Age is the whole of the freshness this phase carries: no threshold hides a
/// fact, because dogfood data is what a threshold would be chosen from
/// (grill Q4).
#[test]
fn age_is_reported_at_the_coarseness_a_person_reads() {
    let cases = [
        (0_u64, "0s ago"),
        (45, "45s ago"),
        (60, "1m ago"),
        (3_599, "59m ago"),
        (3_600, "1h ago"),
        (172_799, "47h ago"),
        (172_800, "2d ago"),
        (864_000, "10d ago"),
    ];
    for (elapsed, expected) in cases {
        let presented = present_at(
            &reported(AgentEventKind::TurnEnded, at(1_000_000)),
            at(1_000_000 + elapsed),
        );
        assert!(
            presented
                .agent
                .as_deref()
                .expect("a line")
                .ends_with(expected),
            "{elapsed}s should read {expected}, got {:?}",
            presented.agent,
        );
    }
}

/// Two clocks disagreeing says nothing about the fact, so a report stamped in
/// the future reads as no time at all rather than as a negative age.
#[test]
fn a_fact_stamped_in_the_future_reads_as_no_age() {
    let presented = present_at(&reported(AgentEventKind::TurnEnded, at(2_000)), at(1_000));

    assert_eq!(
        presented.agent.as_deref(),
        Some("Claude reported finishing a turn · 0s ago"),
    );
}

/// A daemon that reported an event without saying which agent said it has not
/// said enough for a sentence, and a sentence missing its provenance is one
/// this surface must not write.
#[test]
fn a_fact_without_a_provider_renders_nothing() {
    let item = SessionListItem {
        provider: None,
        ..reported(AgentEventKind::TurnEnded, at(1_000))
    };

    assert_eq!(present_at(&item, at(1_010)).agent, None);
}

/// A session with no provider facts is exactly the session PR4 rendered.
#[test]
fn a_session_with_no_provider_facts_says_what_it_always_said() {
    let presented = present(&listed("running", Some(TerminalAccess::Available)));

    assert_eq!(presented.agent, None);
    assert_eq!(presented.beneath(), Vec::<String>::new());
}

/// Both lines, in one order, so the two surfaces cannot arrange the same facts
/// differently.
#[test]
fn the_lines_beneath_are_the_screen_line_then_the_report() {
    let item = SessionListItem {
        terminal_access: Some(TerminalAccess::Unavailable),
        ..reported(AgentEventKind::AwaitingInput, at(1_000))
    };

    let presented = present_at(&item, at(1_060));

    assert_eq!(
        presented.beneath(),
        vec![
            "Screen unavailable".to_owned(),
            "Claude reported waiting for input · 1m ago".to_owned(),
        ],
    );
}

// Snapshot coverage for the rows a person actually reads (workflow §6, the
// mandate that activates at PR7). A hand-written assertion proves the two
// lines its author remembered; a snapshot proves the whole row, and a change
// to any of it has to be looked at.

/// Every line of a row, in the order a surface prints them, as one block.
fn rendered(item: &SessionListItem) -> String {
    let shown = present_at(item, SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
    let mut block = shown.state_line();
    for line in shown.beneath() {
        block.push_str("\n  ");
        block.push_str(&line);
    }
    if let Some(refusal) = shown.refuses_open() {
        block.push_str(&format!("\n  [open refused] {refusal}"));
    }
    block
}

fn external(execution_state: &str, external_id: Option<&str>) -> SessionListItem {
    SessionListItem {
        session_id: "s".to_owned(),
        title: "claude".to_owned(),
        execution_state: execution_state.to_owned(),
        terminal_access: Some(TerminalAccess::Unavailable),
        provider: Some(corral_protocol::method::ProviderFacts {
            name: "claude".to_owned(),
            external_id: external_id.map(str::to_owned),
        }),
        agent_event: None,
        origin: Some(corral_protocol::method::ORIGIN_DISCOVERED.to_owned()),
        location_hint: None,
        attention: None,
    }
}

/// A session Corral found that has not acted yet: it is running, Corral says
/// where it came from, promises no more than that the status resolves when it
/// acts, and refuses Open with the reason already on the row.
#[test]
fn a_discovered_session_without_identity_renders_honestly() {
    insta::assert_snapshot!(rendered(&external("running", None)));
}

/// The same session once a delivery has given it an identity. The origin
/// stays — it is still not Corral's session — and the warming-up line goes,
/// because the session has now acted.
#[test]
fn a_discovered_session_with_identity_drops_the_warming_up_line() {
    insta::assert_snapshot!(rendered(&external("running", Some("session-abc"))));
}

/// A discovered session whose runtime Corral can no longer verify. Execution
/// truth degrades on its own axis and the origin is unaffected.
#[test]
fn a_discovered_session_corral_cannot_verify_renders_honestly() {
    insta::assert_snapshot!(rendered(&external("unknown", None)));
}

/// A session Corral launched says nothing about its origin: labelling the
/// ordinary case would make the exception invisible by making everything a
/// label.
#[test]
fn a_managed_session_carries_no_origin_line() {
    let mut item = external("running", Some("session-abc"));
    item.origin = Some(corral_protocol::method::ORIGIN_MANAGED.to_owned());
    item.terminal_access = Some(TerminalAccess::Available);

    insta::assert_snapshot!(rendered(&item));
}

/// An origin this build has no word for is unknown rather than guessed at —
/// the same rule `execution_state` follows.
#[test]
fn an_origin_from_a_later_build_renders_as_no_origin_at_all() {
    let mut item = external("running", Some("session-abc"));
    item.origin = Some("from-a-later-corral".to_owned());

    insta::assert_snapshot!(rendered(&item));
}

// ---------------------------------------------------------------- attention

fn attended(state: AttentionWireState, items: Vec<AttentionItemFacts>) -> SessionListItem {
    SessionListItem {
        session_id: "s".into(),
        title: "t".into(),
        execution_state: "running".into(),
        terminal_access: None,
        provider: None,
        agent_event: None,
        origin: None,
        location_hint: None,
        attention: Some(AttentionFacts {
            state,
            since_unix_ms: 0,
            last_known: None,
            items,
        }),
    }
}

/// The three semantic states are the main state, alone: the runtime fact
/// sits beside Unknown only (`PRODUCT.md` §4).
#[test]
fn the_daemons_semantic_claim_is_the_main_state() {
    for (state, line) in [
        (AttentionWireState::Working, "Working"),
        (AttentionWireState::NeedsYou, "Needs You"),
        (AttentionWireState::Ready, "Ready"),
    ] {
        let presented = present_at(&attended(state, Vec::new()), SystemTime::UNIX_EPOCH);
        assert_eq!(presented.state_line(), line);
    }
}

/// Unknown from the daemon reads as it always has, with the last reliable
/// fact beneath it in the past tense with its age.
#[test]
fn unknown_keeps_runtime_truth_beside_it_and_the_last_known_fact_beneath() {
    let mut item = attended(AttentionWireState::Unknown, Vec::new());
    item.attention.as_mut().expect("attention").last_known = Some(LastKnownFacts {
        state: AttentionWireState::NeedsYou,
        at_unix_ms: 0,
    });
    let presented = present_at(&item, SystemTime::UNIX_EPOCH + Duration::from_secs(45 * 60));
    assert_eq!(presented.state_line(), "Running · Status unknown");
    assert!(
        presented
            .beneath()
            .contains(&"Last known: Needed input 45m ago".to_owned())
    );
}

/// Exited overrides a cached Needs You: the label is Exited, and the request
/// is neither shown live nor faked as answered.
#[test]
fn exited_before_a_response_says_so() {
    let mut item = attended(AttentionWireState::Exited, Vec::new());
    item.execution_state = "exited".into();
    item.attention.as_mut().expect("attention").last_known = Some(LastKnownFacts {
        state: AttentionWireState::NeedsYou,
        at_unix_ms: 0,
    });
    let presented = present_at(&item, SystemTime::UNIX_EPOCH);
    assert_eq!(presented.state_line(), "Exited");
    assert!(
        presented
            .beneath()
            .contains(&"Exited before you responded".to_owned())
    );
}

/// A state this build cannot name is no claim: the row renders from
/// execution state, as it did before the field existed.
#[test]
fn an_unrecognized_attention_state_renders_no_claim() {
    let item = attended(
        AttentionWireState::Unrecognized("meditating".into()),
        Vec::new(),
    );
    let presented = present_at(&item, SystemTime::UNIX_EPOCH);
    assert_eq!(presented.state_line(), "Running · Status unknown");
}

/// The current item's id is what an acknowledgement will name.
#[test]
fn the_current_item_is_carried_for_acknowledgement() {
    let item = attended(
        AttentionWireState::NeedsYou,
        vec![AttentionItemFacts {
            attention_item_id: "item-1".into(),
            reason: AttentionReasonWire::NeedsInput,
            since_unix_ms: 0,
            acknowledged: false,
        }],
    );
    let presented = present_at(&item, SystemTime::UNIX_EPOCH);
    assert_eq!(presented.acknowledgeable(), Some("item-1"));
    let mut acknowledged = item.clone();
    acknowledged.attention.as_mut().expect("attention").items[0].acknowledged = true;
    assert_eq!(
        present_at(&acknowledged, SystemTime::UNIX_EPOCH).acknowledgeable(),
        None
    );
}
