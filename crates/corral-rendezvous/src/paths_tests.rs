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

mod private_directories {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::test_scratch::scratch_dir;

    fn root_at(dir: &Path, mode: u32) -> RendezvousPaths {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(mode)
            .create(dir)
            .expect("create the root");
        RendezvousPaths::for_corral_root(dir).expect("derivable")
    }

    #[test]
    fn a_freshly_created_tree_is_accepted() {
        let dir = scratch_dir("private-fresh");
        let paths = root_at(&dir.path().join("corral"), 0o700);

        paths.ensure_run_dir().expect("accepted");
        paths.ensure_log_dir().expect("accepted");
    }

    #[test]
    fn a_root_readable_by_the_group_is_refused() {
        let dir = scratch_dir("private-root");
        let paths = root_at(&dir.path().join("corral"), 0o750);

        let error = paths.ensure_run_dir().expect_err("refused");

        assert!(matches!(
            error,
            RendezvousError::RuntimeDirectoryNotPrivate { mode: 0o750, .. }
        ));
    }

    /// A private leaf inside an open root proves nothing: the root is what
    /// keeps another account from replacing the leaf.
    #[test]
    fn an_open_root_is_refused_even_when_the_leaf_is_private() {
        let dir = scratch_dir("private-leaf");
        let root = dir.path().join("corral");
        let paths = root_at(&root, 0o755);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.join("run"))
            .expect("create the leaf");

        let error = paths.ensure_run_dir().expect_err("refused");

        assert!(matches!(
            error,
            RendezvousError::RuntimeDirectoryNotPrivate { mode: 0o755, .. }
        ));
    }

    /// A directory the owner cannot search is not usable either, and saying
    /// so beats the bare EACCES whatever touches it first would produce.
    #[test]
    fn a_directory_the_owner_cannot_search_is_refused() {
        let dir = scratch_dir("private-nosearch");
        let paths = root_at(&dir.path().join("corral"), 0o600);

        let error = paths.ensure_run_dir().expect_err("refused");

        assert!(matches!(
            error,
            RendezvousError::RuntimeDirectoryNotPrivate { mode: 0o600, .. }
        ));
    }

    #[test]
    fn the_log_directory_is_gated_too() {
        let dir = scratch_dir("private-log");
        let root = dir.path().join("corral");
        let paths = root_at(&root, 0o700);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(root.join("log"))
            .expect("create the log dir");

        let error = paths.ensure_log_dir().expect_err("refused");

        assert!(matches!(
            error,
            RendezvousError::RuntimeDirectoryNotPrivate { mode: 0o755, .. }
        ));
        let _ = std::fs::set_permissions(root.join("log"), PermissionsExt::from_mode(0o700));
    }
}
