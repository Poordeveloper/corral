use std::time::SystemTime;

use super::*;
use crate::assurance::Assurance;
use crate::evidence::EvidenceSource;

fn request() -> NeedsInputRequest {
    NeedsInputRequest::new(
        NeedsInputRequestId::mint(),
        CorralSessionId::mint(),
        NeedsInputContext::new(
            ProviderId::new("claude-code").expect("usable"),
            Some(ToolName::new("Bash").expect("usable")),
        ),
        AllowedActions::Exactly(vec![NeedsInputAction::Allow, NeedsInputAction::Deny]),
    )
}

/// Unknown and "no answers accepted" are different facts. A surface may
/// disable its answer path for the second and must not for the first.
#[test]
fn unknown_allowed_actions_are_not_an_empty_list() {
    let unknown = AllowedActions::Unknown;
    let none = AllowedActions::Exactly(Vec::new());

    assert_ne!(unknown, none);
}

#[test]
fn a_request_keeps_the_provider_context_verbatim() {
    let request = request();

    assert_eq!(request.context().provider().as_str(), "claude-code");
    assert_eq!(request.context().tool().map(ToolName::as_str), Some("Bash"));
}

/// A provider that blocked without naming a tool is a normal case, not a
/// broken one.
#[test]
fn a_request_may_name_no_tool() {
    let context = NeedsInputContext::new(ProviderId::new("codex").expect("usable"), None);

    assert_eq!(context.tool(), None);
}

/// The item points at the request, so a surface answers this specific blocked
/// interaction rather than a session-wide flag.
#[test]
fn an_attention_item_can_address_a_specific_request() {
    let request = request();
    let item = AttentionItem::new(
        request.session(),
        AttentionReason::NeedsInput,
        Evidence::new(
            EvidenceSource::ProviderHook,
            Assurance::Attested,
            SystemTime::UNIX_EPOCH,
        ),
        Some(AttentionAction::Answer(request.id())),
    );

    assert_eq!(item.reason(), AttentionReason::NeedsInput);
    assert_eq!(item.action(), Some(&AttentionAction::Answer(request.id())));
}

/// An item Corral cannot name a resolution for is still an item; the absent
/// action is not a claim that nothing would resolve it.
#[test]
fn an_item_without_a_named_action_is_still_an_item() {
    let item = AttentionItem::new(
        CorralSessionId::mint(),
        AttentionReason::TurnComplete,
        Evidence::new(
            EvidenceSource::InBandSignal,
            Assurance::Heuristic,
            SystemTime::UNIX_EPOCH,
        ),
        None,
    );

    assert_eq!(item.action(), None);
    assert_eq!(item.reason(), AttentionReason::TurnComplete);
}
