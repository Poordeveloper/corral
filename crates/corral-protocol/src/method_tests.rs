use std::time::{Duration, SystemTime};

use super::*;
use crate::ErrorCode;
use serde_json::json;

#[test]
fn an_empty_session_list_encodes_as_an_empty_array() {
    let encoded = serde_json::to_string(&SessionListResult::default()).expect("encode");

    assert_eq!(encoded, r#"{"sessions":[]}"#);
}

/// The hand-built wire values must stay the encodings of their types, or
/// the daemon would answer with something the contract does not describe.
#[test]
fn the_hand_built_wire_values_match_their_types() {
    assert_eq!(
        PingResult::wire_value(),
        serde_json::to_value(PingResult::default()).expect("encode")
    );
    assert_eq!(
        SessionListResult::empty_wire_value(),
        serde_json::to_value(SessionListResult::default()).expect("encode")
    );
}

#[test]
fn a_future_session_shape_still_decodes() {
    let decoded: SessionListResult =
        serde_json::from_str(r#"{"sessions":[{"id":"s1","attention":"needs_you"}]}"#)
            .expect("decode");

    assert_eq!(decoded.sessions.len(), 1);
}

#[test]
fn baseline_methods_accept_absent_and_null_params_only() {
    assert!(accepts_no_params(None));
    assert!(accepts_no_params(Some(&Value::Null)));
    assert!(!accepts_no_params(Some(&json!({"workspace": "x"}))));
}

/// `command_id` is required. A request without it is not an old peer being
/// tolerated: without one, a lost response makes a client retry and the retry
/// starts a second agent nobody asked for.
#[test]
fn session_new_without_a_command_id_does_not_decode() {
    let without = json!({ "argv": ["/bin/sh"], "rows": 24, "cols": 80 });

    assert!(serde_json::from_value::<SessionNewParams>(without).is_err());
}

/// Everything else about the shape is additive-tolerant, so a newer client
/// sending a field this build does not know stays decodable.
#[test]
fn session_new_survives_a_field_this_build_does_not_know() {
    let newer = json!({
        "command_id": "cmd-1",
        "argv": ["/bin/sh", "-c", "sleep 30"],
        "cwd": "/work",
        "rows": 24,
        "cols": 80,
        "environment": {"TERM": "xterm"},
    });

    let decoded: SessionNewParams = serde_json::from_value(newer).expect("decode");

    assert_eq!(decoded.command_id, "cmd-1");
    assert_eq!(decoded.argv, ["/bin/sh", "-c", "sleep 30"]);
    assert_eq!(decoded.cwd.as_deref(), Some("/work"));
}

/// The optional fields are absent rather than defaulted to a size Corral would
/// have to invent: absence means the caller has no preference, and a zero is
/// not a geometry.
#[test]
fn session_new_carries_absence_rather_than_a_substituted_size() {
    let minimal = json!({ "command_id": "cmd-1", "argv": ["/bin/sh"] });

    let decoded: SessionNewParams = serde_json::from_value(minimal).expect("decode");

    assert_eq!(decoded.cwd, None);
    assert_eq!(decoded.rows, None);
    assert_eq!(decoded.cols, None);
}

/// The wire spelling is the type's own, in both directions. A daemon that
/// encoded one word and a client that read another would agree in the
/// handshake and disagree about whether a session can be opened.
#[test]
fn terminal_access_round_trips_through_its_wire_spelling() {
    for access in [TerminalAccess::Available, TerminalAccess::Unavailable] {
        assert_eq!(TerminalAccess::from_wire(access.as_str()), Some(access));
        assert_eq!(
            serde_json::to_value(access).expect("encode"),
            json!(access.as_str())
        );
    }
}

/// The absence rule, at the one place it is decided: a peer that does not send
/// the field has said nothing, and nothing is not "unavailable"
/// (AGENTS.md §Protocol).
#[test]
fn a_session_without_terminal_access_decodes_as_unknown() {
    let older = json!({
        "session_id": "s1",
        "title": "sh",
        "execution_state": "running",
    });

    let decoded: SessionListItem = serde_json::from_value(older).expect("decode");

    assert_eq!(decoded.terminal_access, None);
}

/// A value from a version that knows more than this one. It decodes, and it
/// decodes as unknown — refusing the item would lose the session from the
/// list, and reading it as a refusal would disable Open on a word this build
/// never understood.
#[test]
fn an_unrecognised_terminal_access_decodes_as_unknown() {
    for carried in [
        json!("degraded"),
        json!(null),
        json!(7),
        json!({"state": "unavailable"}),
    ] {
        let item = json!({
            "session_id": "s1",
            "title": "sh",
            "execution_state": "running",
            "terminal_access": carried,
        });

        let decoded: SessionListItem =
            serde_json::from_value(item).unwrap_or_else(|error| panic!("{carried}: {error}"));

        assert_eq!(decoded.terminal_access, None, "{carried}");
    }
}

/// Unknown is not encoded. A daemon that cannot say leaves the field out,
/// because a peer reading a spelled-out "unknown" would have to know that
/// word too.
#[test]
fn an_unknown_terminal_access_is_left_off_the_wire() {
    let item = SessionListItem {
        session_id: "s1".to_owned(),
        title: "sh".to_owned(),
        execution_state: "running".to_owned(),
        terminal_access: None,
        provider: None,
        agent_event: None,
        origin: None,
        location_hint: None,
        attention: None,
        last_active_unix_ms: None,
    };

    let encoded = serde_json::to_value(&item).expect("encode");

    assert!(encoded.get("terminal_access").is_none(), "{encoded}");
}

/// A session with no provider facts leaves both fields off the wire. Absence
/// is unknown, and a client must not have to know a spelled-out "unknown".
#[test]
fn absent_provider_facts_are_left_off_the_wire() {
    let item = SessionListItem {
        session_id: "s1".to_owned(),
        title: "sh".to_owned(),
        execution_state: "running".to_owned(),
        terminal_access: None,
        provider: None,
        agent_event: None,
        origin: None,
        location_hint: None,
        attention: None,
        last_active_unix_ms: None,
    };

    let encoded = serde_json::to_value(&item).expect("encode");

    assert!(encoded.get("provider").is_none(), "{encoded}");
    assert!(encoded.get("agent_event").is_none(), "{encoded}");
}

/// An older peer meets a list a newer daemon extended, and keeps reading it.
#[test]
fn a_list_item_without_the_new_fields_still_decodes() {
    let item = json!({
        "session_id": "s1",
        "title": "sh",
        "execution_state": "running",
    });

    let decoded: SessionListItem = serde_json::from_value(item).expect("decode");

    assert!(decoded.provider.is_none());
    assert!(decoded.agent_event.is_none());
}

/// The identity Corral currently stands behind. Withdrawn after a contest, and
/// absence means not currently assertable — never that no id ever existed
/// (ADR 0004 D8).
#[test]
fn a_provider_without_a_current_identity_still_names_the_product() {
    let item = json!({
        "session_id": "s1",
        "title": "claude",
        "execution_state": "running",
        "provider": {"name": "claude"},
    });

    let decoded: SessionListItem = serde_json::from_value(item).expect("decode");

    let provider = decoded.provider.expect("a provider");
    assert_eq!(provider.name, "claude");
    assert_eq!(provider.external_id, None);
}

/// A provider name is data on this wire, not a vocabulary. Adding a product a
/// daemon can start is not a schema change, and a client built before that
/// product existed decodes the name and renders it as it stands — which is
/// what makes `provider.name` safe to extend (ADR 0009; `AGENTS.md`
/// §Protocol).
#[test]
fn a_provider_name_this_build_predates_decodes_as_itself() {
    let item = json!({
        "session_id": "s1",
        "title": "codex",
        "execution_state": "running",
        "provider": {"name": "codex", "external_id": "01a0576f-0ecc-7b21-9719-f38f9e4ef933"},
        "agent_event": {"kind": "turn_ended", "at_ms": 1_700_000_000_000_i64},
    });

    let decoded: SessionListItem = serde_json::from_value(item).expect("decode");

    let provider = decoded.provider.expect("a provider");
    assert_eq!(provider.name, "codex");
    assert_eq!(
        provider.external_id.as_deref(),
        Some("01a0576f-0ecc-7b21-9719-f38f9e4ef933"),
    );
    assert_eq!(
        decoded.agent_event.expect("an event").kind,
        AgentEventKind::TurnEnded,
    );
}

/// Future input: an agent-event kind a newer daemon named decodes rather than
/// failing the item, and keeps its spelling for a diagnostic.
#[test]
fn an_unknown_agent_event_kind_decodes_as_unknown() {
    let item = json!({
        "session_id": "s1",
        "title": "claude",
        "execution_state": "running",
        "agent_event": {"kind": "compacted_context", "at_ms": 1_700_000_000_000_i64},
    });

    let decoded: SessionListItem = serde_json::from_value(item).expect("decode");

    let event = decoded.agent_event.expect("an event");
    assert_eq!(
        event.kind,
        AgentEventKind::Unknown("compacted_context".to_owned()),
    );
    assert_eq!(event.at_ms, 1_700_000_000_000);
}

#[test]
fn every_agent_event_kind_survives_the_wire() {
    for kind in [
        AgentEventKind::SessionStarted,
        AgentEventKind::TurnStarted,
        AgentEventKind::TurnEnded,
        AgentEventKind::AwaitingInput,
        AgentEventKind::SessionEnded,
    ] {
        let encoded = serde_json::to_value(&kind).expect("encode");
        let spelling = encoded.as_str().expect("a string");
        assert_eq!(AgentEventKind::from_wire(spelling), kind);
    }
}

/// `provider` and `argv` are mutually exclusive on the wire, and neither is
/// required — an older peer's request, which always carries `argv`, still
/// means exactly what it always meant.
#[test]
fn session_new_still_decodes_an_older_peers_request() {
    let params = json!({
        "command_id": "c1",
        "argv": ["bash", "-l"],
    });

    let decoded: SessionNewParams = serde_json::from_value(params).expect("decode");

    assert_eq!(decoded.argv, vec!["bash".to_owned(), "-l".to_owned()]);
    assert_eq!(decoded.provider, None);
    assert!(decoded.args.is_empty());
}

#[test]
fn session_new_decodes_the_provider_form() {
    let params = json!({
        "command_id": "c1",
        "provider": "claude",
        "args": ["--model", "opus"],
    });

    let decoded: SessionNewParams = serde_json::from_value(params).expect("decode");

    assert!(decoded.argv.is_empty());
    assert_eq!(decoded.provider.as_deref(), Some("claude"));
    assert_eq!(decoded.args, vec!["--model".to_owned(), "opus".to_owned()]);
}

/// A continuation names the Session, never the provider identity: a caller
/// able to name the provider id would be a caller able to resume an identity
/// Corral does not stand behind.
#[test]
fn session_resume_names_only_a_command_and_a_session() {
    let params = SessionResumeParams {
        command_id: "c1".to_owned(),
        session_id: "s1".to_owned(),
        disclosure_revision: None,
        working_directory: None,
    };

    let encoded = serde_json::to_value(&params).expect("encode");

    assert_eq!(encoded, json!({"command_id": "c1", "session_id": "s1"}),);
}

/// Future input: a `session.resume` from a newer peer decodes past fields
/// this build has not learned, on both sides of the exchange.
#[test]
fn session_resume_decodes_past_unknown_fields() {
    let params = json!({"command_id": "c1", "session_id": "s1", "detach": true});
    let decoded: SessionResumeParams = serde_json::from_value(params).expect("params decode");
    assert_eq!(decoded.command_id, "c1");
    assert_eq!(decoded.session_id, "s1");

    let result = json!({"session_id": "s1", "run_id": "r2", "resumed_at_ms": 0});
    let decoded: SessionResumeResult = serde_json::from_value(result).expect("result decode");
    assert_eq!(decoded.session_id, "s1");
    assert_eq!(decoded.run_id, "r2");
}

/// A secondary fact may not take the row down. The row's promises are its
/// identity, its label, and its execution state; a provider fact is decoration
/// beside them, so a shape this build cannot read degrades that fact to
/// unknown rather than dropping the session out of the list
/// (`AGENTS.md` §Protocol).
#[test]
fn a_secondary_fact_this_build_cannot_read_does_not_lose_the_row() {
    let unreadable = [
        json!({"kind": "turn_ended", "at_ms": 1.5}),
        json!({"kind": 7, "at_ms": 1}),
        json!("turn_ended"),
        json!([]),
    ];
    for carried in unreadable {
        let item = json!({
            "session_id": "s1",
            "title": "claude",
            "execution_state": "running",
            "agent_event": carried,
        });

        let decoded: SessionListItem =
            serde_json::from_value(item).unwrap_or_else(|error| panic!("{carried}: {error}"));

        assert_eq!(decoded.session_id, "s1", "{carried}");
        assert_eq!(decoded.execution_state, "running", "{carried}");
        assert!(decoded.agent_event.is_none(), "{carried}");
    }
}

#[test]
fn a_provider_block_this_build_cannot_read_does_not_lose_the_row() {
    let item = json!({
        "session_id": "s1",
        "title": "claude",
        "execution_state": "running",
        "provider": {"name": ["claude"]},
    });

    let decoded: SessionListItem = serde_json::from_value(item).expect("decode");

    assert_eq!(decoded.session_id, "s1");
    assert!(decoded.provider.is_none());
}

/// A code this build does not know keeps its spelling rather than failing the
/// response, and a code it does know is what behaviour hangs off.
#[test]
fn the_unknown_provider_code_survives_the_wire() {
    let encoded = serde_json::to_value(ErrorCode::UnknownProvider).expect("encode");
    assert_eq!(encoded, json!("unknown_provider"));
    assert_eq!(
        serde_json::from_value::<ErrorCode>(encoded).expect("decode"),
        ErrorCode::UnknownProvider,
    );
}

/// The encoder and the decoder of `at_ms` are one owner, so the sign
/// convention cannot be changed in half of it.
#[test]
fn an_agent_event_survives_its_own_round_trip() {
    for at in [
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_000),
        SystemTime::UNIX_EPOCH - Duration::from_millis(86_400_000),
    ] {
        let event = AgentEvent::at(AgentEventKind::TurnEnded, at).expect("a representable instant");

        assert_eq!(event.observed_at(), at, "{at:?}");
    }
}

/// A clock too far out to describe an age omits the fact rather than
/// saturating into a confident lie.
#[test]
fn an_unrepresentable_instant_produces_no_event() {
    let far = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::MAX / 1_000);

    assert!(AgentEvent::at(AgentEventKind::TurnEnded, far).is_none());
}

// ---------------------------------------------------------------- attention

/// The daemon's claim, when it makes one, rides the row it is about.
#[test]
fn a_list_item_carries_the_daemons_attention_claim() {
    let item = SessionListItem {
        session_id: "s".into(),
        title: "t".into(),
        execution_state: "running".into(),
        terminal_access: None,
        provider: None,
        agent_event: None,
        origin: None,
        location_hint: None,
        attention: Some(AttentionFacts {
            state: AttentionWireState::NeedsYou,
            since_unix_ms: 1_000,
            last_known: None,
            items: vec![AttentionItemFacts {
                attention_item_id: "item-1".into(),
                reason: AttentionReasonWire::NeedsInput,
                since_unix_ms: 1_000,
                acknowledged: false,
            }],
        }),
        last_active_unix_ms: None,
    };
    let encoded = serde_json::to_value(&item).expect("encode");
    assert_eq!(encoded["attention"]["state"], json!("needs_you"));
    assert_eq!(encoded["attention"]["since_unix_ms"], json!(1_000));
    assert_eq!(
        encoded["attention"]["items"][0]["reason"],
        json!("needs_input")
    );
    let decoded: SessionListItem = serde_json::from_value(encoded).expect("decode");
    assert_eq!(
        decoded.attention.expect("attention").state,
        AttentionWireState::NeedsYou
    );
}

/// A state a newer daemon named decodes as no claim: the client renders
/// nothing it cannot name, and keeps the row (`AGENTS.md` §Protocol).
#[test]
fn an_attention_state_this_build_does_not_know_decodes_as_no_claim() {
    let decoded: SessionListItem = serde_json::from_value(json!({
        "session_id": "s", "title": "t", "execution_state": "running",
        "attention": {"state": "meditating", "since_unix_ms": 5, "items": []}
    }))
    .expect("decode");
    let attention = decoded.attention.expect("attention");
    assert_eq!(
        attention.state,
        AttentionWireState::Unrecognized("meditating".into())
    );
    assert_eq!(attention.state.as_claim(), None);
}

/// An older daemon sends no attention object; that is unknown, and the row
/// renders from execution state exactly as before.
#[test]
fn a_list_item_without_attention_still_decodes() {
    let decoded: SessionListItem = serde_json::from_value(json!({
        "session_id": "s", "title": "t", "execution_state": "exited"
    }))
    .expect("decode");
    assert_eq!(decoded.attention, None);
}

/// Unknown carries what was last reliably known, at an instant.
#[test]
fn last_known_rides_beneath_unknown() {
    let facts = AttentionFacts {
        state: AttentionWireState::Unknown,
        since_unix_ms: 9_000,
        last_known: Some(LastKnownFacts {
            state: AttentionWireState::NeedsYou,
            at_unix_ms: 3_000,
        }),
        items: Vec::new(),
    };
    let encoded = serde_json::to_value(&facts).expect("encode");
    assert_eq!(encoded["last_known"]["state"], json!("needs_you"));
    assert_eq!(encoded["last_known"]["at_unix_ms"], json!(3_000));
}

/// The summary is per class, totals and unacknowledged both, so a header
/// and a badge read different numbers from one daemon-owned projection.
#[test]
fn the_summary_carries_totals_and_unacknowledged_per_class() {
    let summary = AttentionSummaryResult {
        needs_you: AttentionCount {
            total: 3,
            unacknowledged: 2,
        },
        ready: AttentionCount {
            total: 1,
            unacknowledged: 0,
        },
    };
    let encoded = serde_json::to_value(summary).expect("encode");
    assert_eq!(
        encoded,
        json!({"needs_you": {"total": 3, "unacknowledged": 2}, "ready": {"total": 1, "unacknowledged": 0}})
    );
    let decoded: AttentionSummaryResult = serde_json::from_value(json!({
        "needs_you": {"total": 3, "unacknowledged": 2}, "ready": {"total": 1, "unacknowledged": 0}, "later": {}
    }))
    .expect("decode past unknown fields");
    assert_eq!(decoded.needs_you.unacknowledged, 2);
}

/// An acknowledgement names the item it saw, never just the session
/// (grill Q18).
#[test]
fn attention_acknowledge_names_a_session_and_an_item() {
    let decoded: AttentionAcknowledgeParams =
        serde_json::from_value(json!({"session_id": "s", "attention_item_id": "i"}))
            .expect("decode");
    assert_eq!(decoded.attention_item_id, "i");
    assert!(
        serde_json::from_value::<AttentionAcknowledgeParams>(json!({"session_id": "s"})).is_err()
    );
}

#[test]
fn the_stale_attention_item_code_survives_the_wire() {
    let encoded = serde_json::to_value(ErrorCode::StaleAttentionItem).expect("encode");
    assert_eq!(encoded, json!("stale_attention_item"));
    let decoded: ErrorCode = serde_json::from_value(encoded).expect("decode");
    assert_eq!(decoded, ErrorCode::StaleAttentionItem);
}

#[test]
fn every_attention_state_and_reason_survives_the_wire() {
    for state in [
        AttentionWireState::Working,
        AttentionWireState::NeedsYou,
        AttentionWireState::Ready,
        AttentionWireState::Unknown,
        AttentionWireState::Exited,
    ] {
        let encoded = serde_json::to_value(&state).expect("encode");
        let decoded: AttentionWireState = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded, state);
    }
    for reason in [
        AttentionReasonWire::NeedsInput,
        AttentionReasonWire::TurnComplete,
    ] {
        let encoded = serde_json::to_value(&reason).expect("encode");
        let decoded: AttentionReasonWire = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded, reason);
    }
}

#[test]
fn the_attention_methods_and_capability_are_named() {
    assert_eq!(ATTENTION_SUMMARY, "attention.summary");
    assert_eq!(ATTENTION_ACKNOWLEDGE, "attention.acknowledge");
    assert_eq!(crate::hello::capability::ATTENTION, "attention.v1");
}

/// The report is the journal read back per day, with the days whose budget
/// ran out named as such rather than counted as quiet (grill Q26).
#[test]
fn the_attention_report_names_incomplete_days() {
    let report = AttentionReportResult {
        days: vec![AttentionDayFacts {
            date: "2026-09-02".into(),
            transitions: 12,
            into_needs_you: 3,
            into_ready: 5,
            disputes: 1,
            incomplete: true,
        }],
    };
    let encoded = serde_json::to_value(&report).expect("encode");
    assert_eq!(encoded["days"][0]["incomplete"], json!(true));
    let decoded: AttentionReportResult = serde_json::from_value(encoded).expect("decode");
    assert_eq!(decoded.days[0].into_needs_you, 3);
    let since: AttentionReportParams =
        serde_json::from_value(json!({"since": "2026-09-01"})).expect("decode");
    assert_eq!(since.since.as_deref(), Some("2026-09-01"));
    let none: AttentionReportParams = serde_json::from_value(json!({})).expect("decode");
    assert_eq!(none.since, None);
}

/// A dispute names the item it is about when the client has one, and the
/// daemon says whether that item was already stale on arrival.
#[test]
fn a_dispute_names_an_item_when_it_can_and_learns_whether_it_was_stale() {
    let params: AttentionDisputeParams =
        serde_json::from_value(json!({"session_id": "s", "attention_item_id": "i"}))
            .expect("decode");
    assert_eq!(params.attention_item_id.as_deref(), Some("i"));
    let bare: AttentionDisputeParams =
        serde_json::from_value(json!({"session_id": "s"})).expect("decode");
    assert_eq!(bare.attention_item_id, None);
    let result = AttentionDisputeResult { stale: true };
    assert_eq!(
        serde_json::to_value(result).expect("encode"),
        json!({"stale": true})
    );
    assert_eq!(ATTENTION_REPORT, "attention.report");
    assert_eq!(ATTENTION_DISPUTE, "attention.dispute");
}

/// A history row says where it came from and when it was last active, and
/// an older peer reads it as an unknown-origin row with unknown execution.
#[test]
fn a_history_row_carries_its_origin_and_recency() {
    assert_eq!(ORIGIN_HISTORY, "history");
    let decoded: SessionListItem = serde_json::from_value(json!({
        "session_id": "s", "title": "Claude Code", "execution_state": "unknown",
        "origin": "history", "last_active_unix_ms": 1_788_350_400_000_i64
    }))
    .expect("decode");
    assert_eq!(decoded.origin.as_deref(), Some(ORIGIN_HISTORY));
    assert_eq!(decoded.last_active_unix_ms, Some(1_788_350_400_000));
    let older: SessionListItem = serde_json::from_value(json!({
        "session_id": "s", "title": "Claude Code", "execution_state": "unknown"
    }))
    .expect("decode");
    assert_eq!(older.last_active_unix_ms, None);
}

// ------------------------------------------------------------ continuation

/// The preflight answers with the daemon's decision, the disclosure it
/// wants shown, and a revision bound to that exact decision (ADR 0016 D5).
#[test]
fn a_continuation_preflight_carries_its_decision_and_revision() {
    assert_eq!(SESSION_CONTINUATION, "session.continuation");
    // The name a client asks before it offers a person this method. A daemon
    // serving `managed-sessions` may predate the whole surface, so nothing
    // else in the hello can answer for it.
    assert_eq!(
        crate::hello::capability::HISTORY_SESSIONS,
        "history-sessions.v1"
    );
    let params: SessionContinuationParams =
        serde_json::from_value(json!({"session_id": "s", "working_directory": "/w"}))
            .expect("decode");
    assert_eq!(params.session_id, "s");
    assert_eq!(params.working_directory.as_deref(), Some("/w"));
    // A client that cannot name its own working directory says nothing rather
    // than naming one, and the daemon refuses what needs one.
    let silent: SessionContinuationParams =
        serde_json::from_value(json!({"session_id": "s"})).expect("decode");
    assert_eq!(silent.working_directory, None);
    let result = SessionContinuationResult {
        code: None,
        decision: CONTINUATION_ELIGIBLE_WITH_DISCLOSURE.to_owned(),
        reason: None,
        disclosure: Some(ContinuationDisclosure {
            code: "history-unknown-live-state".to_owned(),
            text: "Corral can't tell whether this session is still running somewhere else."
                .to_owned(),
        }),
        disclosure_revision: Some("r1".to_owned()),
    };
    let encoded = serde_json::to_value(&result).expect("encode");
    assert_eq!(encoded["decision"], json!("eligible_with_disclosure"));
    assert_eq!(
        encoded["disclosure"]["code"],
        json!("history-unknown-live-state")
    );
    let decoded: SessionContinuationResult = serde_json::from_value(encoded).expect("decode");
    assert_eq!(decoded.disclosure_revision.as_deref(), Some("r1"));
    let refused: SessionContinuationResult = serde_json::from_value(json!({
        "decision": "refused", "reason": "Still running outside Corral."
    }))
    .expect("decode");
    assert_eq!(refused.decision, CONTINUATION_REFUSED);
    assert_eq!(refused.disclosure, None);
    // A daemon that predates the field sends none, and a client that reads
    // none has learned nothing about which refusal this is — not that it is
    // permanent.
    assert_eq!(refused.code, None);

    let busy: SessionContinuationResult = serde_json::from_value(json!({
        "decision": "refused", "code": "busy",
        "reason": "the registry is held by another writer",
        "unknown_field": 1
    }))
    .expect("decode");
    assert_eq!(busy.code.as_deref(), Some("busy"));
}

/// A resume may carry the revision of the disclosure the client showed; an
/// older client sends none, which is how the daemon knows it showed none.
#[test]
fn session_resume_carries_the_disclosure_revision_it_showed() {
    let with: SessionResumeParams = serde_json::from_value(json!({
        "command_id": "c", "session_id": "s", "disclosure_revision": "r1",
        "working_directory": "/w"
    }))
    .expect("decode");
    assert_eq!(with.disclosure_revision.as_deref(), Some("r1"));
    assert_eq!(with.working_directory.as_deref(), Some("/w"));
    let without: SessionResumeParams =
        serde_json::from_value(json!({"command_id": "c", "session_id": "s"})).expect("decode");
    assert_eq!(without.disclosure_revision, None);
    let encoded = serde_json::to_value(&without).expect("encode");
    assert!(encoded.get("disclosure_revision").is_none());
}

#[test]
fn the_stale_disclosure_code_survives_the_wire() {
    let encoded = serde_json::to_value(ErrorCode::StaleDisclosure).expect("encode");
    assert_eq!(encoded, json!("stale_disclosure"));
    let decoded: ErrorCode = serde_json::from_value(encoded).expect("decode");
    assert_eq!(decoded, ErrorCode::StaleDisclosure);
}
