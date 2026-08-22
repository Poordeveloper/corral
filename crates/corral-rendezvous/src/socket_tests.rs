use std::os::unix::net::UnixListener;
use std::time::Duration;

use super::*;
use crate::test_scratch::scratch_dir;

fn claim_in(dir: &Path) -> SingletonClaim {
    SingletonClaim::acquire(&dir.join("corrald.lock"), Duration::from_millis(50))
        .expect("no fault")
        .expect("claimed")
}

#[test]
fn an_absent_pathname_needs_no_cleanup() {
    let dir = scratch_dir("socket-absent");
    let claim = claim_in(dir.path());
    let socket = dir.path().join("corrald.sock");

    assert_eq!(
        inspect_socket_path(&socket).expect("inspect"),
        SocketPathState::Absent
    );
    remove_stale_socket(&claim, &socket).expect("nothing to do");
}

#[test]
fn a_confirmed_socket_artifact_is_removed() {
    let dir = scratch_dir("socket-stale");
    let claim = claim_in(dir.path());
    let socket = dir.path().join("corrald.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    drop(listener);

    assert_eq!(
        inspect_socket_path(&socket).expect("inspect"),
        SocketPathState::SocketArtifact
    );
    remove_stale_socket(&claim, &socket).expect("removed");
    assert!(!socket.exists());
}

#[test]
fn a_regular_file_at_the_endpoint_survives_cleanup() {
    let dir = scratch_dir("socket-regular-file");
    let claim = claim_in(dir.path());
    let socket = dir.path().join("corrald.sock");
    std::fs::write(&socket, b"not a socket").expect("write");

    let error = remove_stale_socket(&claim, &socket).expect_err("fails closed");

    assert!(matches!(
        error,
        RendezvousError::OccupiedSocketPath {
            found: FileKind::RegularFile,
            ..
        }
    ));
    assert!(socket.exists(), "cleanup must never delete a non-socket");
}

#[test]
fn a_symlink_at_the_endpoint_survives_cleanup() {
    let dir = scratch_dir("socket-symlink");
    let claim = claim_in(dir.path());
    let target = dir.path().join("elsewhere");
    std::fs::write(&target, b"target").expect("write");
    let socket = dir.path().join("corrald.sock");
    std::os::unix::fs::symlink(&target, &socket).expect("symlink");

    let error = remove_stale_socket(&claim, &socket).expect_err("fails closed");

    assert!(matches!(
        error,
        RendezvousError::OccupiedSocketPath {
            found: FileKind::Symlink,
            ..
        }
    ));
    assert!(target.exists(), "cleanup must never follow a substitution");
}

#[test]
fn a_directory_at_the_endpoint_survives_cleanup() {
    let dir = scratch_dir("socket-directory");
    let claim = claim_in(dir.path());
    let socket = dir.path().join("corrald.sock");
    std::fs::create_dir(&socket).expect("mkdir");

    let error = remove_stale_socket(&claim, &socket).expect_err("fails closed");

    assert!(matches!(
        error,
        RendezvousError::OccupiedSocketPath {
            found: FileKind::Directory,
            ..
        }
    ));
    assert!(socket.is_dir());
}
