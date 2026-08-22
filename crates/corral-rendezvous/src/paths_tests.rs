use super::*;

#[test]
fn derives_every_artifact_from_the_account_home() {
    let paths = RendezvousPaths::for_account_home("/home/example").expect("derivable");

    assert_eq!(
        paths.socket(),
        Path::new("/home/example/.corral/run/corrald.sock")
    );
    assert_eq!(
        paths.lock(),
        Path::new("/home/example/.corral/run/corrald.lock")
    );
    assert_eq!(paths.run_dir(), Path::new("/home/example/.corral/run"));
    assert_eq!(
        paths.log_file(),
        Path::new("/home/example/.corral/log/corrald.log")
    );
}

/// The account home only supplies the root; the layout under it is one
/// rule, so a test namespace differs from a real account in nothing else.
#[test]
fn the_account_home_only_supplies_the_root() {
    let from_home = RendezvousPaths::for_account_home("/home/example").expect("derivable");
    let from_root = RendezvousPaths::for_corral_root("/home/example/.corral").expect("ok");

    assert_eq!(from_home, from_root);
}

#[test]
fn derivation_depends_on_nothing_but_the_root() {
    let a = RendezvousPaths::for_account_home("/home/example").expect("derivable");
    let b = RendezvousPaths::for_account_home("/home/example/").expect("derivable");
    assert_eq!(a, b);
}

#[test]
fn a_canonical_path_too_long_for_a_socket_is_a_configuration_error() {
    let deep = format!("/{}", "d".repeat(200));
    let error = RendezvousPaths::for_account_home(&deep).expect_err("too long");
    assert!(matches!(
        error,
        RendezvousError::CanonicalEndpointTooLong { .. }
    ));
}

/// A test namespace is subject to the same limits as a real root: the seam
/// substitutes the root and nothing else.
#[test]
fn a_test_namespace_gets_the_same_validation_as_a_real_root() {
    let deep = format!("/{}", "d".repeat(200));
    let error = RendezvousPaths::for_corral_root(&deep).expect_err("too long");
    assert!(matches!(
        error,
        RendezvousError::CanonicalEndpointTooLong { .. }
    ));
}

#[test]
fn an_empty_override_is_rejected_without_falling_back() {
    let error = validate_endpoint_path(OsStr::new("")).expect_err("empty");
    assert!(matches!(
        error,
        RendezvousError::InvalidExplicitEndpoint {
            reason: InvalidEndpointReason::Empty,
            ..
        }
    ));
}

#[test]
fn a_relative_override_is_rejected_without_falling_back() {
    let error = validate_endpoint_path(OsStr::new("run/corrald.sock")).expect_err("relative");
    assert!(matches!(
        error,
        RendezvousError::InvalidExplicitEndpoint {
            reason: InvalidEndpointReason::Relative,
            ..
        }
    ));
}

#[test]
fn an_oversized_override_is_rejected_without_falling_back() {
    let raw = format!("/{}", "x".repeat(400));
    let error = validate_endpoint_path(OsStr::new(&raw)).expect_err("too long");
    assert!(matches!(
        error,
        RendezvousError::InvalidExplicitEndpoint {
            reason: InvalidEndpointReason::TooLong,
            ..
        }
    ));
}

#[test]
fn a_usable_override_is_returned_unchanged() {
    let path = validate_endpoint_path(OsStr::new("/tmp/corral-test.sock")).expect("usable");
    assert_eq!(path, Path::new("/tmp/corral-test.sock"));
}
