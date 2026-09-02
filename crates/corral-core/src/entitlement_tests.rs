use super::*;
use crate::assurance::Assurance;
use crate::evidence::EvidenceSource;

fn claim(source: EvidenceSource, asserts: SemanticState) -> Claim {
    Claim {
        source,
        association: Assurance::Attested,
        channel: Channel::CorralOwnedPty,
        sealing: Sealing::Sealed,
        asserts,
    }
}

/// The association axis comes first: evidence over a Heuristic binding is
/// secondary metadata whatever its source says (AGENTS.md §Core model).
#[test]
fn a_heuristic_association_asserts_nothing() {
    for source in [
        EvidenceSource::ProviderHook,
        EvidenceSource::ScreenDetection,
        EvidenceSource::PtyActivity,
    ] {
        let mut weak = claim(source, SemanticState::NeedsYou);
        weak.association = Assurance::Heuristic;
        assert_eq!(
            weak.entitlement(),
            Entitlement::AssociationTooWeak,
            "{source:?}"
        );
    }
}

/// PTY activity says the agent is drawing, and only that (ADR 0015 D5).
#[test]
fn pty_activity_asserts_only_working() {
    assert_eq!(
        claim(EvidenceSource::PtyActivity, SemanticState::Working).entitlement(),
        Entitlement::Entitled
    );
    assert_eq!(
        claim(EvidenceSource::PtyActivity, SemanticState::NeedsYou).entitlement(),
        Entitlement::SourceMayNotAssert
    );
    assert_eq!(
        claim(EvidenceSource::PtyActivity, SemanticState::Ready).entitlement(),
        Entitlement::SourceMayNotAssert
    );
}

/// An external runtime has no stream Corral reads, so nothing about its
/// screen or activity is Corral's to claim.
#[test]
fn screen_and_activity_exist_only_on_a_corral_owned_pty() {
    for (source, state) in [
        (EvidenceSource::PtyActivity, SemanticState::Working),
        (EvidenceSource::ScreenDetection, SemanticState::NeedsYou),
        (EvidenceSource::InBandSignal, SemanticState::NeedsYou),
    ] {
        let mut external = claim(source, state);
        external.channel = Channel::ExternalRuntime;
        assert_eq!(
            external.entitlement(),
            Entitlement::NotCorralOwned,
            "{source:?}"
        );
    }
}

/// A sealed screen rule may assert Needs You or Ready; a Working rule stays
/// diagnostic in this phase (grill Q14); an unsealed rule asserts nothing.
#[test]
fn a_screen_rule_asserts_what_its_seal_covers_and_never_working() {
    assert_eq!(
        claim(EvidenceSource::ScreenDetection, SemanticState::NeedsYou).entitlement(),
        Entitlement::Entitled
    );
    assert_eq!(
        claim(EvidenceSource::ScreenDetection, SemanticState::Ready).entitlement(),
        Entitlement::Entitled
    );
    assert_eq!(
        claim(EvidenceSource::ScreenDetection, SemanticState::Working).entitlement(),
        Entitlement::SourceMayNotAssert
    );
    let mut unsealed = claim(EvidenceSource::ScreenDetection, SemanticState::NeedsYou);
    unsealed.sealing = Sealing::Unsealed;
    assert_eq!(unsealed.entitlement(), Entitlement::Unsealed);
}

/// A received, attested, version-sealed provider event is sufficient for
/// exactly the claim it denotes (grill Q2); unsealed, it is diagnostics.
#[test]
fn a_sealed_provider_event_asserts_exactly_what_it_denotes() {
    for state in [
        SemanticState::Working,
        SemanticState::NeedsYou,
        SemanticState::Ready,
    ] {
        let mut hook = claim(EvidenceSource::ProviderHook, state);
        hook.channel = Channel::ExternalRuntime;
        assert_eq!(hook.entitlement(), Entitlement::Entitled, "{state:?}");
        hook.sealing = Sealing::Unsealed;
        assert_eq!(hook.entitlement(), Entitlement::Unsealed, "{state:?}");
    }
}

/// An in-band sequence asserts what its sealed matrix row says, on the PTY
/// Corral owns.
#[test]
fn a_sealed_in_band_signal_asserts_its_sealed_meaning() {
    for state in [
        SemanticState::Working,
        SemanticState::NeedsYou,
        SemanticState::Ready,
    ] {
        assert_eq!(
            claim(EvidenceSource::InBandSignal, state).entitlement(),
            Entitlement::Entitled,
            "{state:?}"
        );
    }
    let mut unsealed = claim(EvidenceSource::InBandSignal, SemanticState::NeedsYou);
    unsealed.sealing = Sealing::Unsealed;
    assert_eq!(unsealed.entitlement(), Entitlement::Unsealed);
}

/// Runtime observation, construction, history, correlation, and a person's
/// link say nothing about what the agent is doing now.
#[test]
fn non_semantic_sources_never_assert_a_main_state() {
    for source in [
        EvidenceSource::CorralConstructed,
        EvidenceSource::NodeRuntimeObservation,
        EvidenceSource::HistoryRecord,
        EvidenceSource::Correlation,
        EvidenceSource::UserAssertion,
    ] {
        for state in [
            SemanticState::Working,
            SemanticState::NeedsYou,
            SemanticState::Ready,
        ] {
            assert_eq!(
                claim(source, state).entitlement(),
                Entitlement::SourceMayNotAssert,
                "{source:?} {state:?}"
            );
        }
    }
}
