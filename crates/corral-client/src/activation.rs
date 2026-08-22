use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use corral_rendezvous::{OwnerProbe, RendezvousPaths, probe_owner};
use tokio::net::UnixStream;

use crate::connection::{Connection, handshake};
use crate::endpoint::EndpointSelection;
use crate::error::{ActivationContext, ActivationError};
use crate::policy::ClientActivationPolicy;
use crate::spawn::{SpawnedDaemon, spawn_daemon};

/// How often the machine retries while a daemon is starting or exiting.
const RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// Obtain a usable connection to this account's `corrald`, starting one if the
/// canonical rendezvous has no owner.
///
/// Success means connected *and* handshaken: reachability alone is not
/// readiness, so a listener that never completes a hello is a failure with a
/// name, not a connection.
pub async fn activate(policy: &ClientActivationPolicy) -> Result<Connection, ActivationError> {
    let deadline = Instant::now() + policy.activation_deadline;

    match EndpointSelection::from_environment()? {
        EndpointSelection::Explicit(endpoint) => connect_explicit(&endpoint).await,
        EndpointSelection::Canonical(paths) => {
            paths.ensure_run_dir()?;
            activate_canonical(&paths, policy, deadline).await
        }
    }
}

/// An externally managed endpoint: connect or fail, never start anything.
async fn connect_explicit(endpoint: &Path) -> Result<Connection, ActivationError> {
    let stream = UnixStream::connect(endpoint).await.map_err(|source| {
        ActivationError::ExplicitEndpointUnavailable {
            endpoint: endpoint.to_path_buf(),
            source,
        }
    })?;

    match handshake(stream, endpoint, ActivationContext::ExistingPrimary).await? {
        Some(connection) => Ok(connection),
        // Nothing here may start a replacement, so a daemon that closed
        // mid-bootstrap is simply unavailable.
        None => Err(ActivationError::ExplicitEndpointUnavailable {
            endpoint: endpoint.to_path_buf(),
            source: io::Error::from(io::ErrorKind::ConnectionAborted),
        }),
    }
}

async fn activate_canonical(
    paths: &RendezvousPaths,
    policy: &ClientActivationPolicy,
    deadline: Instant,
) -> Result<Connection, ActivationError> {
    let mut spawned: Option<SpawnedDaemon> = None;

    loop {
        let context = if spawned.is_some() {
            ActivationContext::ActivationAttempted
        } else {
            ActivationContext::ExistingPrimary
        };

        match UnixStream::connect(paths.socket()).await {
            // A daemon that closes during the bootstrap was on its way out.
            // Its lock is about to be released, so the remaining deadline is
            // exactly the right thing to spend here.
            Ok(stream) => {
                if let Some(mut connection) = handshake(stream, paths.socket(), context).await? {
                    connection.attach_daemon(spawned);
                    return Ok(connection);
                }
            }
            Err(source) if activation_may_repair(&source) => {}
            Err(source) => {
                return Err(ActivationError::Endpoint {
                    endpoint: paths.socket().to_path_buf(),
                    source,
                });
            }
        }

        if Instant::now() >= deadline {
            return Err(give_up(paths, policy, spawned.as_mut()));
        }

        // Absence of a socket says nothing about whether starting a daemon is
        // safe; only absence of a lock owner does.
        if spawned.is_none() && probe_owner(paths.lock())? == OwnerProbe::NoOwner {
            spawned = Some(spawn_daemon(paths)?);
        }

        tokio::time::sleep(RETRY_INTERVAL).await;
    }
}

/// Connect failures that a daemon appearing would fix.
///
/// Everything else — a permission fault, a path that is not a socket — is a
/// configuration problem that retrying cannot repair.
fn activation_may_repair(source: &io::Error) -> bool {
    matches!(
        source.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            // A listen backlog that is momentarily full reports as
            // would-block on some systems.
            | io::ErrorKind::WouldBlock
    )
}

fn give_up(
    paths: &RendezvousPaths,
    policy: &ClientActivationPolicy,
    spawned: Option<&mut SpawnedDaemon>,
) -> ActivationError {
    match spawned {
        Some(daemon) => ActivationError::SpawnedDaemonDidNotBecomeReady {
            endpoint: paths.socket().to_path_buf(),
            deadline: policy.activation_deadline,
            spawn_result: daemon.outcome(),
        },
        // Never spawning means every probe found an owner: a primary daemon
        // exists and its rendezvous is unusable.
        None => ActivationError::OwnerPresentButUnreachable {
            lock_path: paths.lock().to_path_buf(),
            endpoint: paths.socket().to_path_buf(),
            deadline: policy.activation_deadline,
        },
    }
}
