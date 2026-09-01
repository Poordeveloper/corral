use super::*;

#[test]
fn withdrawn_authority_never_permits_a_repair() {
    let withdrawn = RepairAuthority::Withdrawn {
        since: SystemTime::UNIX_EPOCH,
    };
    assert!(!withdrawn.permits_repair());
}

#[test]
fn an_exhausted_budget_permits_no_repair_before_the_breaker_opens() {
    assert!(RepairAuthority::Available { remaining: 1 }.permits_repair());
    assert!(!RepairAuthority::Available { remaining: 0 }.permits_repair());
}

#[test]
fn the_window_bound_is_the_budgets_span_before_now() {
    let day = Duration::from_secs(24 * 60 * 60);
    let budget = RepairBudget::new(3, day);
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 24 * 60 * 60);

    assert_eq!(budget.window_starts(now), now - day);
}

#[test]
fn one_drift_class_does_not_spend_anothers_budget() {
    let provider = ProviderId::new("claude").expect("a supported provider name");
    let missing = RepairFingerprint::new(
        provider.clone(),
        ConfigTarget::ClaudeUserSettings,
        RepairableDrift::Missing,
    );
    let stale = RepairFingerprint::new(
        provider,
        ConfigTarget::ClaudeUserSettings,
        RepairableDrift::OldRepresentation,
    );

    assert_ne!(missing, stale);
}
