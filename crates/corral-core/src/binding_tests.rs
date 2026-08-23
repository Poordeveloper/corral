use std::time::SystemTime;

use super::*;
use crate::evidence::EvidenceSource;

fn binding(kind: BindingKind, assurance: Assurance) -> Binding {
    Binding::new(
        BindingId::mint(),
        CorralSessionId::mint(),
        BindingKey::new(
            NodeId::mint(),
            kind,
            ProviderId::new("claude-code").expect("usable"),
            ExternalId::new("abc-123").expect("usable"),
        ),
        Provenance::Discovered,
        Evidence::new(
            EvidenceSource::NodeRuntimeObservation,
            assurance,
            SystemTime::UNIX_EPOCH,
        ),
        SystemTime::UNIX_EPOCH,
    )
}

#[test]
fn a_heuristic_binding_is_never_control_eligible() {
    let weak = binding(BindingKind::Runtime, Assurance::Heuristic);

    assert_eq!(
        weak.control_eligibility(),
        ControlEligibility::AssuranceTooWeak
    );
    assert!(!weak.is_control_capable_runtime_binding());
}

#[test]
fn a_strong_runtime_binding_is_control_capable() {
    for assurance in [
        Assurance::Deterministic,
        Assurance::Attested,
        Assurance::Manual,
    ] {
        let strong = binding(BindingKind::Runtime, assurance);
        assert_eq!(strong.control_eligibility(), ControlEligibility::Eligible);
        assert!(strong.is_control_capable_runtime_binding());
    }
}

/// The at-most-one rule is about runtime bindings. A deterministic history or
/// provider-session binding is control eligible in its own right and does not
/// consume that slot.
#[test]
fn only_runtime_bindings_occupy_the_control_capable_runtime_slot() {
    for kind in [
        BindingKind::ProviderSession,
        BindingKind::Terminal,
        BindingKind::History,
    ] {
        let other = binding(kind, Assurance::Deterministic);
        assert_eq!(other.control_eligibility(), ControlEligibility::Eligible);
        assert!(!other.is_control_capable_runtime_binding());
    }
}

/// Assurance is re-evaluated when evidence changes, and eligibility follows
/// it immediately — no separate stamp to keep in sync.
#[test]
fn eligibility_follows_the_current_evidence() {
    let weak = binding(BindingKind::Runtime, Assurance::Heuristic);
    let confirmed = weak.clone().with_evidence(Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        SystemTime::UNIX_EPOCH,
    ));

    assert_eq!(
        weak.control_eligibility(),
        ControlEligibility::AssuranceTooWeak
    );
    assert_eq!(
        confirmed.control_eligibility(),
        ControlEligibility::Eligible
    );
    assert_eq!(confirmed.id(), weak.id(), "confirming is not re-binding");
}

/// The Session is not part of the key: the key is what a Session is looked up
/// by, so the same external identity cannot resolve to two Sessions.
#[test]
fn the_uniqueness_key_excludes_the_session() {
    let one = binding(BindingKind::Runtime, Assurance::Deterministic);
    let two = Binding::new(
        BindingId::mint(),
        CorralSessionId::mint(),
        one.key().clone(),
        Provenance::Discovered,
        one.evidence(),
        SystemTime::UNIX_EPOCH,
    );

    assert_ne!(one.session(), two.session());
    assert_eq!(one.key(), two.key());
}
