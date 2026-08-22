use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use crate::error::{FileKind, RendezvousError};
use crate::lock::SingletonClaim;

/// What occupies the socket pathname.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketPathState {
    Absent,
    /// A Unix socket artifact. Under a held singleton claim no other daemon
    /// can be listening on it, which is what makes it stale.
    SocketArtifact,
    Occupied(FileKind),
}

/// Inspect the socket pathname without following a symlink into it.
///
/// A symlink at the endpoint is reported as an occupant rather than resolved,
/// because resolving it would let a substitution decide where the daemon binds.
pub fn inspect_socket_path(path: &Path) -> Result<SocketPathState, RendezvousError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SocketPathState::Absent);
        }
        Err(source) => {
            return Err(RendezvousError::SocketPathname {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let file_type = metadata.file_type();
    Ok(if file_type.is_socket() {
        SocketPathState::SocketArtifact
    } else if file_type.is_symlink() {
        SocketPathState::Occupied(FileKind::Symlink)
    } else if file_type.is_dir() {
        SocketPathState::Occupied(FileKind::Directory)
    } else if file_type.is_file() {
        SocketPathState::Occupied(FileKind::RegularFile)
    } else {
        SocketPathState::Occupied(FileKind::Other)
    })
}

/// Remove a socket pathname left behind by a dead daemon.
///
/// Taking the claim by reference is the ownership proof: only the exclusive
/// lock winner may clean, because only it knows no other daemon is serving
/// there. Cleanup is not a file-deletion primitive — anything that is not a
/// socket artifact fails closed and stays on disk (ADR 0001 D3).
pub fn remove_stale_socket(claim: &SingletonClaim, path: &Path) -> Result<(), RendezvousError> {
    debug_assert!(claim.path().parent() == path.parent());

    match inspect_socket_path(path)? {
        SocketPathState::Absent => Ok(()),
        SocketPathState::Occupied(found) => Err(RendezvousError::OccupiedSocketPath {
            path: path.to_path_buf(),
            found,
        }),
        SocketPathState::SocketArtifact => {
            std::fs::remove_file(path).map_err(|source| RendezvousError::SocketPathname {
                path: path.to_path_buf(),
                source,
            })
        }
    }
}

#[cfg(test)]
mod tests {
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
}
