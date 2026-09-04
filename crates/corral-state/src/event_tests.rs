use std::time::Duration;

use corral_core::{
    Assurance, BindingKind, CommandFingerprint, CommandKind, EvidenceSource, ExitCause, Provenance,
};

use super::*;

fn instant(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn session() -> CorralSessionId {
    CorralSessionId::mint()
}

fn binding(session: CorralSessionId) -> Binding {
    Binding::new(
        BindingId::mint(),
        session,
        BindingKey::new(
            corral_core::NodeId::mint(),
            BindingKind::Runtime,
            ProviderId::new("claude-code").expect("usable"),
            ExternalId::new("sess-9").expect("usable"),
        ),
        Provenance::Discovered,
        Evidence::new(
            EvidenceSource::NodeRuntimeObservation,
            Assurance::Heuristic,
            instant(30),
        ),
        instant(31),
    )
}

fn every_event(session: CorralSessionId) -> Vec<SessionEvent> {
    let binding = binding(session);
    let run = RunId::mint();
    vec![
        SessionEvent::SessionCreated {
            session,
            created_at: instant(10),
        },
        SessionEvent::BindingAdded(binding.clone()),
        SessionEvent::BindingConfirmed {
            session,
            binding: binding.id(),
            evidence: Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(40),
            ),
        },
        SessionEvent::BindingContested {
            session,
            binding: binding.id(),
            conflicting_external_id: ExternalId::new("sess-10").expect("usable"),
            evidence: Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(46),
            ),
        },
        SessionEvent::RunStarted {
            session,
            run,
            runtime_binding: binding.id(),
            started_at: Some(instant(41)),
            working_directory: Some(std::path::PathBuf::from("/w")),
        },
        SessionEvent::RunStarted {
            session,
            run,
            runtime_binding: binding.id(),
            started_at: None,
            working_directory: None,
        },
        SessionEvent::RunAttached {
            session,
            run,
            at: instant(42),
        },
        SessionEvent::RunDetached {
            session,
            run,
            at: instant(43),
        },
        SessionEvent::RunEnded {
            session,
            run,
            end: RunEnd::Exited(ExitCause::Completed),
            ended_at: Some(instant(44)),
        },
        SessionEvent::RunEnded {
            session,
            run,
            end: RunEnd::Unverifiable,
            ended_at: None,
        },
        SessionEvent::SessionForkedFrom(
            SessionLineage::record(session, CorralSessionId::mint(), Assurance::Deterministic)
                .expect("recordable"),
        ),
        SessionEvent::CommandAccepted {
            command: CommandId::new("cmd-1").expect("usable"),
            fingerprint: CommandFingerprint::builder(
                CommandKind::new("session.create").expect("usable"),
            )
            .input("cwd", "/work")
            .build(),
            outcome: CommandOutcome::SessionCreated(session),
            accepted_at: instant(45),
        },
        SessionEvent::CommandAccepted {
            command: CommandId::new("cmd-2").expect("usable"),
            fingerprint: CommandFingerprint::builder(
                CommandKind::new("session.resume").expect("usable"),
            )
            .input("session", session.to_string())
            .build(),
            outcome: CommandOutcome::RunStarted { session, run },
            accepted_at: instant(47),
        },
    ]
}

#[test]
fn every_fact_round_trips_through_its_stored_form() {
    let session = session();

    for event in every_event(session) {
        let payload = encode(&event).expect("encodable");
        let decoded = decode(event.session(), event.kind(), &payload).expect("decodable");

        assert_eq!(decoded, event, "{} did not round trip", event.kind());
    }
}

/// The Session id is a column of the log, so no payload carries a second copy
/// to disagree with it.
///
/// `command-accepted` is the one payload that names a Session, and it names
/// the Session the command *produced*: that is the receipt's outcome, read
/// back without walking the log, and it is what `session()` derives the stream
/// from — so the two cannot disagree.
#[test]
fn the_payload_never_repeats_the_session_id() {
    let session = session();

    for event in every_event(session) {
        let payload = encode(&event).expect("encodable");
        assert!(
            payload.get("session_id").is_none(),
            "{} carries a session_id field",
            event.kind()
        );
        if matches!(event, SessionEvent::CommandAccepted { .. }) {
            continue;
        }
        assert!(
            !payload.to_string().contains(&session.to_string()),
            "{} wrote its own session id into the payload",
            event.kind()
        );
    }
}

/// Every fact carries a distinct durable name, or two kinds of fact would
/// decode as one.
#[test]
fn each_kind_of_fact_has_its_own_durable_name() {
    let session = session();
    let mut names: Vec<&str> = every_event(session)
        .iter()
        .map(SessionEvent::kind)
        .collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();

    // Two `RunStarted`, two `RunEnded`, and two `CommandAccepted` appear
    // above, each pair covering a shape of the same fact.
    assert_eq!(names.len(), total - 3);
}

/// An absent occurrence time is a recorded absence — the fact happened, and
/// the runtime could not say when — never a zero instant.
#[test]
fn an_absent_occurrence_time_decodes_as_absent() {
    let session = session();
    let event = SessionEvent::RunStarted {
        session,
        run: RunId::mint(),
        runtime_binding: BindingId::mint(),
        started_at: None,
        working_directory: None,
    };

    let payload = encode(&event).expect("encodable");
    let decoded = decode(session, event.kind(), &payload).expect("decodable");

    assert!(matches!(
        decoded,
        SessionEvent::RunStarted {
            started_at: None,
            ..
        }
    ));
}

/// A `run-started` written before Runs carried a directory decodes as one
/// whose directory is unknown.
///
/// The same answer a Run Corral found gets, and deliberately so: both mean
/// Corral cannot say where the episode ran, which refuses a continuation
/// rather than choosing a directory for it (Q35).
#[test]
fn a_run_started_without_a_directory_decodes_as_unknown() {
    let session = session();
    let older = serde_json::json!({
        "run_id": RunId::mint().to_string(),
        "runtime_binding_id": BindingId::mint().to_string(),
        "started_at_ms": 41_000,
    });

    let decoded = decode(session, "run-started", &older).expect("decodable");

    assert!(matches!(
        decoded,
        SessionEvent::RunStarted {
            working_directory: None,
            ..
        }
    ));
}

/// A fact written by a build that knows more kinds than this one is not
/// skipped: projections derived from a log with an unread event would be
/// silently incomplete.
#[test]
fn an_unknown_kind_of_fact_is_unreadable_rather_than_skipped() {
    let error = decode(session(), "session-archived", &json!({})).expect_err("unreadable");

    assert!(matches!(error, FatalState::Unreadable { .. }));
}

/// A payload may gain a field without the fact changing meaning, so an older
/// build reads what it knows and ignores the rest. Refusing would make a store
/// unreadable by the build that wrote it the moment anything is added.
#[test]
fn an_unknown_payload_field_is_ignored() {
    let session = session();

    for event in every_event(session) {
        let mut payload = encode(&event).expect("encodable");
        payload["a_field_a_later_build_added"] = json!({"nested": [1, 2, 3]});

        let decoded = decode(event.session(), event.kind(), &payload).expect("decodable");

        assert_eq!(
            decoded,
            event,
            "{} did not survive an added field",
            event.kind()
        );
    }
}

#[test]
fn a_payload_missing_a_field_is_unreadable() {
    let error = decode(session(), "session-created", &json!({})).expect_err("unreadable");

    assert!(matches!(error, FatalState::Unreadable { .. }));
}

/// A fork is recorded on the child, so the new Session's own stream states
/// where it came from.
#[test]
fn a_fork_belongs_to_the_child_stream() {
    let child = session();
    let parent = session();
    let event = SessionEvent::SessionForkedFrom(
        SessionLineage::record(child, parent, Assurance::Deterministic).expect("recordable"),
    );

    assert_eq!(event.session(), child);
}

/// The one new fact this phase adds, and what it means: this identifier was
/// reported, and nothing more. It creates no binding to it, merges no
/// Sessions, and does not assert the conflicting id is the correct one
/// (ADR 0004 D8).
#[test]
fn a_contest_records_the_reported_identifier_and_creates_nothing() {
    let session = session();
    let binding = binding(session);
    let contested = SessionEvent::BindingContested {
        session,
        binding: binding.id(),
        conflicting_external_id: ExternalId::new("the-other-one").expect("usable"),
        evidence: Evidence::new(
            EvidenceSource::ProviderHook,
            Assurance::Attested,
            instant(50),
        ),
    };

    let payload = encode(&contested).expect("encodable");

    assert_eq!(contested.kind(), "binding-contested");
    assert_eq!(payload["conflicting_external_id"], "the-other-one");
    assert_eq!(payload["binding_id"], binding.id().to_string());
    // No provider, no kind, no provenance: a binding is what those fields
    // build, and this fact builds none.
    for absent in ["provider", "kind", "provenance", "node_id", "external_id"] {
        assert!(payload.get(absent).is_none(), "{absent} in {payload}");
    }
}

/// Future input, the fail-closed half: a kind this build cannot interpret
/// would leave every projection derived from the log silently incomplete.
#[test]
fn a_recorded_kind_this_build_does_not_know_is_unreadable() {
    let refusal = decode(session(), "binding-uncontested", &json!({}));

    assert!(
        matches!(refusal, Err(FatalState::Unreadable { .. })),
        "{refusal:?}"
    );
}

/// A continuation's receipt names the Run it made. Reading one back without it
/// would answer a retry with the wrong episode, so it fails closed rather than
/// guessing.
#[test]
fn a_run_started_receipt_without_its_run_is_unreadable() {
    let refusal = decode(
        session(),
        "command-accepted",
        &json!({
            "command_id": "cmd-2",
            "fingerprint": "k14:session.resume",
            "outcome_kind": "started-run",
            "outcome_target": session().to_string(),
            "accepted_at_ms": 47_000,
        }),
    );

    assert!(
        matches!(refusal, Err(FatalState::Unreadable { .. })),
        "{refusal:?}"
    );
}
