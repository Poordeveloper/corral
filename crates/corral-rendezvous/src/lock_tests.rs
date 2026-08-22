use super::*;
use crate::test_scratch::{permission_checks_apply, scratch_dir};

#[test]
fn an_unclaimed_lock_probes_as_having_no_owner() {
    let dir = scratch_dir("lock-free");
    let lock = dir.path().join("corrald.lock");

    assert_eq!(probe_owner(&lock).expect("probe"), OwnerProbe::NoOwner);
}

#[test]
fn a_held_claim_probes_as_owner_present() {
    let dir = scratch_dir("lock-held");
    let lock = dir.path().join("corrald.lock");

    let claim = SingletonClaim::acquire(&lock, Duration::from_millis(50))
        .expect("no fault")
        .expect("claimed");

    assert_eq!(probe_owner(&lock).expect("probe"), OwnerProbe::OwnerPresent);
    drop(claim);
    assert_eq!(probe_owner(&lock).expect("probe"), OwnerProbe::NoOwner);
}

#[test]
fn a_second_claim_loses_the_race_rather_than_failing() {
    let dir = scratch_dir("lock-race");
    let lock = dir.path().join("corrald.lock");

    let _winner = SingletonClaim::acquire(&lock, Duration::from_millis(50))
        .expect("no fault")
        .expect("claimed");
    let loser = SingletonClaim::acquire(&lock, Duration::from_millis(50)).expect("no fault");

    assert!(loser.is_none());
}

#[test]
fn a_probe_never_reports_a_permission_fault_as_an_owner() {
    if !permission_checks_apply() {
        return;
    }
    let dir = scratch_dir("lock-eacces");
    let closed = dir.path().join("closed");
    std::fs::create_dir(&closed).expect("create");
    std::fs::set_permissions(&closed, std::os::unix::fs::PermissionsExt::from_mode(0o000))
        .expect("chmod");

    let error = probe_owner(&closed.join("corrald.lock")).expect_err("permission fault");

    std::fs::set_permissions(&closed, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .expect("restore");
    assert!(matches!(error, RendezvousError::Lock { .. }));
}
