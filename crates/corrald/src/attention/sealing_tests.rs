use super::*;

fn claim(provider: KnownProvider, fact: AgentFactKind, version: Option<&str>) -> Option<Claim> {
    hook_fact_claim(
        provider,
        fact,
        version,
        Assurance::Attested,
        Channel::CorralOwnedPty,
    )
}

/// Sealing is exact and per version, and it covers only the facts that were
/// measured on that version. A version nobody measured is Unsealed, which is
/// Limited awareness, not inherited authority (grill Q13, Q28).
#[test]
fn a_fact_is_sealed_for_the_version_it_was_measured_on_and_no_other() {
    for fact in [
        AgentFactKind::TurnStarted,
        AgentFactKind::TurnEnded,
        AgentFactKind::AwaitingInput,
    ] {
        assert_eq!(
            claim(KnownProvider::Claude, fact, Some("2.1.258"))
                .expect("a turn fact makes a claim")
                .sealing,
            Sealing::Sealed,
            "{fact:?}"
        );
        for unmeasured in ["2.1.259", "2.1.252", "2.1.260", ""] {
            assert_eq!(
                claim(KnownProvider::Claude, fact, Some(unmeasured))
                    .expect("a turn fact makes a claim")
                    .sealing,
                Sealing::Unsealed,
                "{fact:?} on {unmeasured}"
            );
        }
        // A runtime whose version could not be established seals nothing.
        assert_eq!(
            claim(KnownProvider::Claude, fact, None)
                .expect("a turn fact makes a claim")
                .sealing,
            Sealing::Unsealed,
            "{fact:?}"
        );
    }
}

/// Codex reports one fact and one only: a turn completed. It has no turn-start
/// notify and no approval notify, so those two are sealed for nothing — an
/// adapter that later invented them would not inherit this row.
#[test]
fn codex_seals_the_one_fact_its_notify_carries() {
    assert_eq!(
        claim(
            KnownProvider::Codex,
            AgentFactKind::TurnEnded,
            Some("0.152.0")
        )
        .expect("a turn fact makes a claim")
        .sealing,
        Sealing::Sealed
    );
    for unmeasured in [AgentFactKind::TurnStarted, AgentFactKind::AwaitingInput] {
        assert_eq!(
            claim(KnownProvider::Codex, unmeasured, Some("0.152.0"))
                .expect("a turn fact makes a claim")
                .sealing,
            Sealing::Unsealed,
            "{unmeasured:?}"
        );
    }
    assert_eq!(
        claim(
            KnownProvider::Codex,
            AgentFactKind::TurnEnded,
            Some("0.145.0")
        )
        .expect("a turn fact makes a claim")
        .sealing,
        Sealing::Unsealed
    );
}

/// A start and an end are not turn-state claims whatever the version.
#[test]
fn a_session_boundary_claims_no_turn_state() {
    for fact in [AgentFactKind::SessionStarted, AgentFactKind::SessionEnded] {
        assert_eq!(claim(KnownProvider::Claude, fact, Some("2.1.258")), None);
    }
}
