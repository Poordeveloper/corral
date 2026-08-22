use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use corral_rendezvous::{OwnerProbe, RendezvousPaths, probe_owner};
use tokio::net::UnixStream;

use crate::connection::{Connection, handshake};
use crate::endpoint::EndpointSelection;
use crate::error::{ActivationContext, ActivationError, OwnerEvidence};
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
        EndpointSelection::Explicit(endpoint) => connect_explicit(&endpoint, deadline).await,
        EndpointSelection::Canonical(paths) => {
            paths.ensure_run_dir()?;
            activate_canonical(&paths, policy, deadline).await
        }
    }
}

/// An externally managed endpoint: connect or fail, never start anything.
async fn connect_explicit(
    endpoint: &Path,
    deadline: Instant,
) -> Result<Connection, ActivationError> {
    let unavailable = |source| ActivationError::ExplicitEndpointUnavailable {
        endpoint: endpoint.to_path_buf(),
        source,
    };

    // The budget covers the handshake too. A peer that accepts the connection
    // and never answers is precisely what an overall deadline exists for.
    let attempt = async {
        let stream = UnixStream::connect(endpoint).await.map_err(unavailable)?;
        handshake(stream, endpoint, ActivationContext::ExistingPrimary).await
    };

    match within(deadline, attempt).await {
        Some(Ok(Some(connection))) => Ok(connection),
        // Nothing here may start a replacement, so a daemon that closed
        // mid-bootstrap is simply unavailable.
        Some(Ok(None)) => Err(unavailable(io::Error::from(
            io::ErrorKind::ConnectionAborted,
        ))),
        Some(Err(error)) => Err(error),
        None => Err(unavailable(io::Error::from(io::ErrorKind::TimedOut))),
    }
}

async fn activate_canonical(
    paths: &RendezvousPaths,
    policy: &ClientActivationPolicy,
    deadline: Instant,
) -> Result<Connection, ActivationError> {
    let mut spawned: Option<SpawnedDaemon> = None;
    // What the client has actually established about a primary owner. Derived
    // facts would be guesses, and a guess reported as a runtime fact is the
    // one thing activation may never do.
    let mut owner = OwnerEvidence::NotProbed;

    loop {
        let context = if spawned.is_some() {
            ActivationContext::ActivationAttempted
        } else {
            ActivationContext::ExistingPrimary
        };

        // Connect and handshake share the remaining budget with everything
        // else. Leaving the readiness step unbounded would leave the one
        // stage the deadline exists for outside it.
        match within(deadline, connect_and_handshake(paths, context)).await {
            Some(Ok(Some(mut connection))) => {
                connection.attach_daemon(spawned);
                return Ok(connection);
            }
            // A daemon that closes during the bootstrap was on its way out.
            // Its lock is about to be released, so the remaining deadline is
            // exactly the right thing to spend here.
            Some(Ok(None)) => {}
            Some(Err(error)) => return Err(error),
            None => return Err(give_up(paths, policy, spawned.as_mut(), owner)),
        }

        if Instant::now() >= deadline {
            return Err(give_up(paths, policy, spawned.as_mut(), owner));
        }

        // Absence of a socket says nothing about whether starting a daemon is
        // safe; only absence of a lock owner does.
        if spawned.is_none() {
            owner = match probe_owner(paths.lock())? {
                OwnerProbe::OwnerPresent => OwnerEvidence::Present,
                OwnerProbe::NoOwner => {
                    spawned = Some(spawn_daemon(paths)?);
                    OwnerEvidence::Absent
                }
            };
        }

        // Clamped, so the deadline the failure reports is the one the caller
        // actually waited.
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(RETRY_INTERVAL.min(remaining)).await;
    }
}

/// One attempt at the canonical endpoint.
///
/// `Ok(None)` means "not usable yet, and a daemon appearing would fix it".
async fn connect_and_handshake(
    paths: &RendezvousPaths,
    context: ActivationContext,
) -> Result<Option<Connection>, ActivationError> {
    match UnixStream::connect(paths.socket()).await {
        Ok(stream) => handshake(stream, paths.socket(), context).await,
        Err(source) if activation_may_repair(&source) => Ok(None),
        Err(source) => Err(ActivationError::Endpoint {
            endpoint: paths.socket().to_path_buf(),
            source,
        }),
    }
}

/// Run `work` with whatever is left of the overall budget.
///
/// `None` means the budget ran out. An already-expired deadline still refuses
/// rather than granting one free unbounded attempt.
async fn within<T>(deadline: Instant, work: impl Future<Output = T>) -> Option<T> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    tokio::time::timeout(remaining, work).await.ok()
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
    owner: OwnerEvidence,
) -> ActivationError {
    if let Some(daemon) = spawned {
        return ActivationError::SpawnedDaemonDidNotBecomeReady {
            endpoint: paths.socket().to_path_buf(),
            deadline: policy.activation_deadline,
            spawn_result: daemon.outcome(),
        };
    }
    match owner {
        // Observed, not inferred: a probe reported an owner and the endpoint
        // never became usable.
        OwnerEvidence::Present => ActivationError::OwnerPresentButUnreachable {
            lock_path: paths.lock().to_path_buf(),
            endpoint: paths.socket().to_path_buf(),
            deadline: policy.activation_deadline,
        },
        // The budget ran out before the rendezvous could be assessed, or
        // after a probe found it free. Claiming an owner in either case
        // would invent a fact nothing observed.
        evidence @ (OwnerEvidence::NotProbed | OwnerEvidence::Absent) => {
            ActivationError::ActivationBudgetExpired {
                endpoint: paths.socket().to_path_buf(),
                deadline: policy.activation_deadline,
                owner: evidence,
            }
        }
    }
}
