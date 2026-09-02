use std::time::SystemTime;

use super::*;
use crate::evidence::EvidenceSource;

fn binding(kind: BindingKind, assurance: Assurance) -> Binding {
    owned_binding(kind, assurance, Provenance::CorralCreated)
}

fn owned_binding(kind: BindingKind, assurance: Assurance, provenance: Provenance) -> Binding {
    Binding::new(
        BindingId::mint(),
        CorralSessionId::mint(),
        BindingKey::new(
            NodeId::mint(),
            kind,
            ProviderId::new("claude-code").expect("usable"),
            ExternalId::new("abc-123").expect("usable"),
        ),
        provenance,
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

/// A binding is born as the identity Corral stands behind. Contest is a later
/// fact about it, never a shape it can be created in (ADR 0004 D8).
#[test]
fn a_binding_is_created_confirmed() {
    let fresh = binding(BindingKind::ProviderSession, Assurance::Attested);

    assert_eq!(fresh.identity_status(), IdentityStatus::Confirmed);
    assert_eq!(
        fresh.native_resume_eligibility(),
        NativeResumeEligibility::Eligible
    );
}

/// Attested-and-contested is not Heuristic. Assurance records how the
/// association was learned; a contest says which external identity it names
/// became ambiguous, and collapsing them would misdescribe the fact.
#[test]
fn a_contest_leaves_assurance_and_evidence_untouched() {
    let attested = binding(BindingKind::ProviderSession, Assurance::Attested);
    let contested = attested.clone().contested();

    assert_eq!(contested.assurance(), Assurance::Attested);
    assert_eq!(contested.evidence(), attested.evidence());
    assert_eq!(contested.id(), attested.id());
    assert_eq!(contested.key(), attested.key());
    assert_eq!(contested.identity_status(), IdentityStatus::Contested);
}

/// Contested revokes exactly the authority derived from the identity claim.
/// Generic binding control is untouched, because Open and terminal attach ride
/// the Deterministic runtime binding and are honestly still supported
/// (ADR 0004 D8; founder emphasis on R2 Q2).
#[test]
fn a_contest_revokes_native_resume_and_nothing_else() {
    let contested = binding(BindingKind::ProviderSession, Assurance::Attested).contested();

    assert_eq!(
        contested.native_resume_eligibility(),
        NativeResumeEligibility::IdentityContested
    );
    assert_eq!(
        contested.control_eligibility(),
        ControlEligibility::Eligible
    );
}

/// A contested binding keeps the assurance it earned, so the refusal has to
/// name the contest rather than report the weaker-sounding reason.
#[test]
fn a_contested_binding_is_not_reported_as_too_weak() {
    let contested = binding(BindingKind::ProviderSession, Assurance::Heuristic).contested();

    assert_eq!(
        contested.native_resume_eligibility(),
        NativeResumeEligibility::IdentityContested
    );
}

#[test]
fn heuristic_evidence_never_continues_a_provider_session() {
    let weak = binding(BindingKind::ProviderSession, Assurance::Heuristic);

    assert_eq!(
        weak.native_resume_eligibility(),
        NativeResumeEligibility::AssuranceTooWeak
    );
}

/// Every assurance level has a stated answer, so a level added later has to be
/// decided rather than fall through a wildcard.
#[test]
fn every_assurance_level_answers_native_resume() {
    for assurance in [
        Assurance::Deterministic,
        Assurance::Attested,
        Assurance::Manual,
    ] {
        assert_eq!(
            binding(BindingKind::ProviderSession, assurance).native_resume_eligibility(),
            NativeResumeEligibility::Eligible,
            "{assurance:?}",
        );
    }
    assert_eq!(
        binding(BindingKind::ProviderSession, Assurance::Heuristic).native_resume_eligibility(),
        NativeResumeEligibility::AssuranceTooWeak,
    );
}

/// Monotonic in this phase: nothing here returns a contested edge to
/// confirmed, and contesting one twice changes nothing.
#[test]
fn contesting_a_contested_binding_changes_nothing() {
    let once = binding(BindingKind::ProviderSession, Assurance::Attested).contested();
    let twice = once.clone().contested();

    assert_eq!(once, twice);
}

/// A runtime binding Corral did not create names a process Corral holds no
/// handle on. It is read-only by structure, not by weak evidence: corroborating
/// it harder does not make it Corral's to drive (ADR 0014 D6).
#[test]
fn a_discovered_runtime_is_never_control_capable_however_strong() {
    for assurance in [
        Assurance::Deterministic,
        Assurance::Attested,
        Assurance::Manual,
    ] {
        let discovered = owned_binding(BindingKind::Runtime, assurance, Provenance::Discovered);

        // The evidence is strong enough; the ownership is what is missing, and
        // the two answers stay separate.
        assert_eq!(
            discovered.control_eligibility(),
            ControlEligibility::Eligible
        );
        assert!(!discovered.is_control_capable_runtime_binding());
    }
}

/// A user's explicit link is the case that does grant it, which is why the
/// rule names provenance rather than "Corral created it" (`PRODUCT.md` §6).
#[test]
fn a_runtime_the_user_linked_is_control_capable() {
    let linked = owned_binding(
        BindingKind::Runtime,
        Assurance::Manual,
        Provenance::UserLinked,
    );

    assert!(linked.is_control_capable_runtime_binding());
}
