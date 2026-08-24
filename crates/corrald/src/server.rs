use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, error, info};

use crate::connection;
use crate::lifecycle::{Lifecycle, Phase, ShutdownReason, watch_idle};
use crate::policy::DaemonPolicy;
use crate::state::DaemonState;

/// Keeps a failing accept from spinning the CPU while the cause persists.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// Bind, serve, and return once shutdown has run to completion.
///
/// The caller still holds the singleton claim when this returns, so the window
/// between the last client closing and the process exiting still reads as
/// "owner present" to anyone probing — which is what stops a second daemon
/// from starting into a half-dismantled rendezvous.
pub async fn serve(socket: &Path, policy: DaemonPolicy, state: Arc<DaemonState>) -> io::Result<()> {
    let listener = UnixListener::bind(socket)?;
    // The run directory is already user-private; the socket says so too rather
    // than inheriting whatever the umask happened to be.
    std::fs::set_permissions(socket, PermissionsExt::from_mode(0o600))?;

    let lifecycle = Lifecycle::new(Instant::now());
    let mut shutdown = lifecycle.subscribe();

    let counted = Arc::clone(&state);
    tokio::spawn(watch_idle(
        Arc::clone(&lifecycle),
        policy.idle_grace,
        move || counted.live_sessions(),
    ));
    tokio::spawn(watch_signals(Arc::clone(&lifecycle)));

    info!(endpoint = %socket.display(), "corrald is serving");

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _address)) => {
                    tokio::spawn(connection::serve(
                        stream,
                        Arc::clone(&lifecycle),
                        Arc::clone(&state),
                        policy,
                        lifecycle.subscribe(),
                    ));
                }
                Err(source) => {
                    error!(%source, "accept failed");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
        }
    }

    // Committed: stop accepting first, so nothing new can arrive while the
    // established connections are being closed.
    drop(listener);
    debug!(
        reason = ?lifecycle.shutdown_reason(),
        established = lifecycle.established_clients(),
        "corrald is shutting down"
    );
    // Named one at a time, not counted. A managed run ending because the
    // daemon did is the one shutdown consequence worth being able to see
    // afterwards, and a number cannot tell anyone which run it was
    // (ADR 0007 L6). Corral does not wait for these children and does not
    // reap them, so it never claims they exited — their terminals are hung up
    // by the kernel when this process closes the last descriptor of each.
    for session in state.running_sessions() {
        info!(
            session = %session.session,
            run = %session.run,
            title = %session.title,
            "a managed run is ending because corrald is",
        );
    }
    lifecycle.mark_exited();
    debug_assert_eq!(lifecycle.phase(), Phase::Exited);
    Ok(())
}

/// SIGTERM and SIGINT enter the same committed path as an idle exit; the only
/// difference is that they commit immediately, whatever is connected.
async fn watch_signals(lifecycle: Arc<Lifecycle>) {
    let (mut terminate, mut interrupt) = match (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) {
        (Ok(terminate), Ok(interrupt)) => (terminate, interrupt),
        _ => {
            error!("signal handlers could not be installed");
            return;
        }
    };

    let reason = tokio::select! {
        _ = terminate.recv() => ShutdownReason::Signal("SIGTERM"),
        _ = interrupt.recv() => ShutdownReason::Signal("SIGINT"),
    };
    info!(?reason, "shutting down on a signal");
    lifecycle.commit_shutdown(reason);
}
