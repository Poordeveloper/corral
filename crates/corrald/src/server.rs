use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tracing::{debug, error, info};

use crate::connection;
use crate::lifecycle::{Lifecycle, Phase, ShutdownReason, watch_idle};
use crate::policy::DaemonPolicy;
use crate::state::DaemonState;

/// Accept connections until shutdown, serving each one and no more than
/// `CONCURRENT_CONNECTIONS` of them at a time.
///
/// One loop for both of this daemon's listeners. They had the same shape and
/// the same two decisions — what to do about a failing accept, and how many
/// connections may be in flight — and two copies of a decision is how the two
/// come to differ. What differs is only what a connection *is*, which is the
/// argument.
///
/// The bound is held by a permit the serving task owns, so a slot frees when
/// the connection ends however it ends. While none is free this stops calling
/// `accept`, which leaves the pending connections in the kernel's backlog
/// rather than answering them with a close.
pub(crate) async fn accept_until_shutdown<Serve, Serving>(
    listener: &UnixListener,
    shutdown: &mut watch::Receiver<bool>,
    what: &'static str,
    serve_one: Serve,
) where
    Serve: Fn(tokio::net::UnixStream) -> Serving,
    Serving: std::future::Future<Output = ()> + Send + 'static,
{
    let slots = Arc::new(tokio::sync::Semaphore::new(
        crate::policy::CONCURRENT_CONNECTIONS,
    ));
    loop {
        // Taken before the accept, so a daemon at its bound waits here rather
        // than accepting a connection it has nowhere to serve.
        let slot = tokio::select! {
            _ = shutdown.changed() => break,
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                // Only a closed semaphore, which nothing here does.
                Err(_) => break,
            },
        };
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _address)) => {
                    let serving = serve_one(stream);
                    tokio::spawn(async move {
                        let _slot = slot;
                        serving.await;
                    });
                }
                Err(source) => {
                    error!(%source, "accepting {what} failed");
                    tokio::time::sleep(crate::policy::ACCEPT_BACKOFF).await;
                }
            },
        }
    }
}

/// Bind, serve, and return once shutdown has run to completion.
///
/// The caller still holds the singleton claim when this returns, so the window
/// between the last client closing and the process exiting still reads as
/// "owner present" to anyone probing — which is what stops a second daemon
/// from starting into a half-dismantled rendezvous.
pub async fn serve(
    socket: &Path,
    hook_socket: &Path,
    policy: DaemonPolicy,
    state: Arc<DaemonState>,
) -> io::Result<()> {
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
    tokio::spawn(watch_run_lifecycle(
        Arc::clone(&lifecycle),
        state.observations().watch_integrity(),
    ));

    // One task interprets every delivered hook event, in the order they
    // arrived. Serial on purpose: two drainers would let two events race to
    // establish one Session's first provider identity (ADR 0004 D5).
    if let Some(incoming) = state.take_deliveries() {
        tokio::spawn(crate::hook_evidence::ingest(Arc::clone(&state), incoming));
    }
    // A hook endpoint that will not bind costs awareness, never the daemon:
    // the sessions this process owns keep running, and every relay that cannot
    // reach it fails open in milliseconds. What it does cost is the ability to
    // start a *managed* session — one whose hooks deliver here — so the answer
    // is recorded rather than only logged, and bound here rather than inside
    // the task so no client can ask before it is known.
    let hook_socket = hook_socket.to_path_buf();
    match crate::hook_endpoint::bind(&hook_socket) {
        Ok(hook_listener) => {
            let deliveries = state.deliveries();
            let hook_shutdown = lifecycle.subscribe();
            tokio::spawn(async move {
                crate::hook_endpoint::serve(&hook_socket, hook_listener, deliveries, hook_shutdown)
                    .await;
            });
        }
        Err(source) => {
            error!(%source, endpoint = %hook_socket.display(), "the hook endpoint could not serve");
            state.hook_endpoint_unavailable();
        }
    }

    // At startup, and nowhere on a timer: drift repair is a boundary
    // operation, never a background normalization loop (ADR 0013 D5). It runs
    // after the endpoint work because a provider file Corral cannot repair
    // costs awareness, never the daemon — and only for a provider whose
    // integration the user actually chose.
    tokio::spawn(crate::integration::repair_at_startup(Arc::clone(&state)));

    // The no-lying reconciliation law, with external sessions in its scope
    // (ADR 0014 D5): a Run this node recorded before must not still be shown
    // as running because the daemon restarted. Every one is re-verified, and
    // a process this account cannot inspect ends `Unverifiable` rather than
    // exited — unreachable is never stopped.
    tokio::spawn(crate::external_session::reverify_external_runs(Arc::clone(
        &state,
    )));

    info!(endpoint = %socket.display(), "corrald is serving");

    let accepting = Arc::clone(&lifecycle);
    let served = Arc::clone(&state);
    accept_until_shutdown(&listener, &mut shutdown, "a client", move |stream| {
        connection::serve(
            stream,
            Arc::clone(&accepting),
            Arc::clone(&served),
            policy,
            accepting.subscribe(),
        )
    })
    .await;

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

/// Stop serving once a run lifecycle fact has been lost.
///
/// A queue that could not take an observation, or a fact the store would not
/// write, means this daemon can no longer account for the runs it owns. That
/// is not backpressure to ride out: it is the same conclusion as a store that
/// cannot vouch, reached by another route, and it takes the same fail-closed
/// path (grill Q10).
async fn watch_run_lifecycle(
    lifecycle: Arc<Lifecycle>,
    mut integrity: tokio::sync::watch::Receiver<crate::runtime::Integrity>,
) {
    loop {
        // Read before waiting. The recorder is started with the store, before
        // anything is served and before startup reconciliation runs, so a
        // fact already lost by the time this task exists would otherwise be
        // waited past — a subscriber begins having seen the current value.
        if *integrity.borrow_and_update() == crate::runtime::Integrity::Lost {
            error!("a managed run's lifecycle could not be recorded; corrald is stopping");
            lifecycle.commit_shutdown(ShutdownReason::FatalState);
            return;
        }
        if integrity.changed().await.is_err() {
            return;
        }
    }
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
